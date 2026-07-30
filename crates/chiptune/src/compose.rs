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
use crate::theory::{Chord, Ext, Mode, NOTE_NAMES, euclid_rot};
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
        if (step as u32).is_multiple_of(self.steps_per_beat()) {
            0
        } else if step.is_multiple_of(2) {
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
    /// Zen at terminal gravity. Same bright, nothing-at-stake harmony as
    /// [`Profile::Zen`] — the mode is still unlosable — but the floor has
    /// dropped out from under it: a fast, four-on-the-floor pulse that
    /// makes the last stretch of a zen run feel like the reward it is.
    ZenPeak = 5,
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

/// The one progression here that is not a walk between plain triads: what
/// Japanese pop calls the 丸サ (maru-sa) progression, after the song it is
/// named for, and what jazz would call a set of changes — `IVM7 III7 vi I7`,
/// the loop under a very large slice of the last twenty years of J-pop and
/// vocaloid writing.
///
/// Two things make it work and both are outside the key. Its `III7` and
/// its `I7` are secondary dominants: they pull to the chord that follows
/// instead of merely arriving next to it, which is why the loop feels like
/// it is going somewhere despite never leaving.
///
/// It is stored here in its minor reading and rotated to start on the
/// tonic — `i7 bIII7 bVIM7 V7`, which is the same four chords in the same
/// order — because that puts a real dominant on the fourth bar, exactly
/// where [`build_material`] schedules its half cadence. The rotation costs
/// nothing musically and saves the progression from having to argue with
/// the cadence machinery.
///
/// In A minor that reads `Am7 C7 FM7 E7`.
const MARUSA: [Chord; 4] = [
    Chord::with(0, Ext::Seventh),
    Chord::dom(2, Ext::Seventh),
    Chord::with(5, Ext::Seventh),
    Chord::dom(4, Ext::Seventh),
];

/// The maru-sa changes only spell themselves in a mode with a minor sixth
/// and a minor third — its `bVIM7` is a major seventh chord built on the
/// flat sixth, which Dorian and the bright modes simply do not have. So a
/// piece that uses it plays in plain minor and stays there.
///
/// That costs the intensity ladder its say over the mode for those pieces.
/// It is a fair trade: the progression is doing the harmonic work that the
/// ladder would otherwise be doing, and the arrangement still thickens
/// with the danger.
const MARUSA_MODE: Mode = Mode::Aeolian;

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

/// A strum: which step, and whether the hand is going down or up.
///
/// Down means low string first, up means high string first, and an
/// upstroke only catches the top of the chord — a real one does not
/// reach the bass strings on the way back. That asymmetry is most of
/// what separates a strummed chord from a chord played six times.
#[derive(Clone, Copy)]
struct Strum {
    step: u8,
    down: bool,
}

const fn d(step: u8) -> Strum {
    Strum { step, down: true }
}
const fn u(step: u8) -> Strum {
    Strum { step, down: false }
}

/// `D - D U - U D U` on the eighths: the pattern every guitarist learns
/// first, and the reason is that it works.
const STRUM_4_4: [Strum; 6] = [d(0), d(4), u(6), u(10), d(12), u(14)];
const STRUM_3_4: [Strum; 5] = [d(0), d(4), u(6), d(8), u(10)];
/// 6/8 is two groups of three, so the hand moves on the dotted beat.
const STRUM_6_8: [Strum; 4] = [d(0), u(3), d(6), u(9)];
/// The sparse version, for when the chords are background rather than
/// the point: one downstroke a beat, no upstrokes at all.
const STRUM_SLOW_4_4: [Strum; 4] = [d(0), d(4), d(8), d(12)];
const STRUM_SLOW_3_4: [Strum; 3] = [d(0), d(4), d(8)];
const STRUM_SLOW_6_8: [Strum; 2] = [d(0), d(6)];

fn strum_pattern(meter: Meter, busy: bool) -> &'static [Strum] {
    match (meter, busy) {
        (Meter::Four, true) => &STRUM_4_4,
        (Meter::Four, false) => &STRUM_SLOW_4_4,
        (Meter::Three, true) => &STRUM_3_4,
        (Meter::Three, false) => &STRUM_SLOW_3_4,
        (Meter::Six, true) => &STRUM_6_8,
        (Meter::Six, false) => &STRUM_SLOW_6_8,
    }
}

/// Seconds between one string and the next inside a single strum. A
/// gesture, not a musical duration, so it does not scale with tempo —
/// a guitarist's hand crosses the strings at the same speed at 80 BPM
/// as at 200.
const STRUM_SPREAD_SECS: f32 = 0.013;

/// How the chord blocks play their chord.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChordStyle {
    /// Three voices held together, organ-like.
    Sustain,
    /// Struck string by string, in a strum pattern.
    Strum,
}

impl ChordStyle {
    pub fn name(self) -> &'static str {
        match self {
            ChordStyle::Sustain => "held",
            ChordStyle::Strum => "strum",
        }
    }

    pub const ALL: [ChordStyle; 2] = [ChordStyle::Sustain, ChordStyle::Strum];
}

/// What a block of the form is for.
///
/// This is the piece's shape, and it is the thing that was missing: the
/// arrangement used to be a pure function of intensity and elapsed time,
/// so it only ever thickened. Nothing ever stopped. A tune that never
/// takes the melody away has no way to bring it back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BlockRole {
    /// Everything the layer schedule allows.
    Full,
    /// The melody sits out and all three pulses hold a real chord
    /// instead. The block where the harmony is the point.
    Chords,
    /// The melody sits out and almost everything goes with it — bass,
    /// pad and a bare kick. The air before the tune comes back.
    Break,
}

/// The section orders a piece can be built out of.
///
/// Every one of them opens and closes on A, uses all three sections, and
/// gives A more than half the blocks — the return of the first thing you
/// heard is what a listener is actually tracking over eighty bars, and
/// the closing block is the final chorus, which wants the main tune
/// rather than whatever B was doing.
///
/// The lengths differ on purpose. Two pieces with different material but
/// the same eighty-bar clock still feel like the same song shape; a
/// twelve-block form is a genuinely longer arc, and a session that hears
/// both is hearing two pieces rather than one piece twice.
const FORMS: [&[usize]; 6] = [
    // A A B A C A B A C A — statement, departure, statement.
    &[0, 0, 1, 0, 2, 0, 1, 0, 2, 0],
    // A B A B C A B A C A — verse/chorus alternation from the top.
    &[0, 1, 0, 1, 2, 0, 1, 0, 2, 0],
    // A A B B A C C A B A — sections in pairs, which lets B and C settle
    // instead of passing through.
    &[0, 0, 1, 1, 0, 2, 2, 0, 1, 0],
    // A B B A C A A B C A
    &[0, 1, 1, 0, 2, 0, 0, 1, 2, 0],
    // Twelve blocks: the long arc, with C held back until the middle.
    &[0, 0, 1, 0, 1, 2, 0, 2, 1, 0, 2, 0],
    &[0, 1, 0, 2, 0, 1, 1, 0, 2, 2, 1, 0],
];

/// How often a piece strums its chord blocks rather than holding them.
/// The solo profiles lean strummed: those are the pieces built around
/// the harmony, and a strum gives the block its own pulse instead of
/// leaving it to the drums.
fn strum_chance(profile: Profile) -> f32 {
    match profile {
        Profile::SoloCalm => 0.65,
        Profile::Zen => 0.55,
        Profile::Ambient => 0.40,
        Profile::VsIntense => 0.45,
        Profile::ZenPeak => 0.45,
        Profile::Victory => 0.50,
    }
}

/// How many blocks of the form hand themselves over to the harmony,
/// inclusive. Versus and the fanfare stay driven: a match is not the
/// place to keep stopping the melody. The solo and menu profiles put the
/// chords out front instead, so the harmony is something the piece is
/// *about* rather than a place it visits twice.
fn chord_budget(profile: Profile) -> (i32, i32) {
    match profile {
        // Zen's peak is a zen piece that has started moving; it keeps the
        // drive rather than stopping twice to admire the chords.
        Profile::VsIntense | Profile::Victory | Profile::ZenPeak => (1, 2),
        Profile::SoloCalm | Profile::Zen | Profile::Ambient => (3, 4),
    }
}

/// Give one block away to `role`, out of the candidates in `from`.
///
/// Specials are spread out where there is room: a block that still has
/// the tune between two that do not is what makes the tune's return
/// audible. Two in a row is allowed only once nothing else is left,
/// which is the only reason this can't just reject and retry.
fn place_role(role: BlockRole, from: &[usize], roles: &mut [BlockRole], rng: &mut Rng) {
    let open: Vec<usize> = from
        .iter()
        .copied()
        .filter(|&i| roles[i] == BlockRole::Full)
        .collect();
    let spread: Vec<usize> = open
        .iter()
        .copied()
        .filter(|&i| roles[i - 1] == BlockRole::Full && roles[i + 1] == BlockRole::Full)
        .collect();
    let pool = if spread.is_empty() { &open } else { &spread };
    if pool.is_empty() {
        return;
    }
    roles[*rng.pick(pool)] = role;
}

/// Roll where this piece stops playing the tune.
///
/// This used to be one of two hand-written tables, so every versus piece
/// in the game broke down in the same two bars. The shape of the form is
/// the loudest thing about a piece after its key, and it was the one
/// thing that never varied.
fn roll_roles(profile: Profile, blocks: usize, rng: &mut Rng) -> Vec<BlockRole> {
    let mut roles = vec![BlockRole::Full; blocks];
    let (lo, hi) = chord_budget(profile);
    let chords = rng.range(lo, hi) as usize;
    // A second breakdown only in the long form; in the short one it
    // costs a third of the melody.
    let breaks = if blocks >= 12 && rng.chance(0.4) {
        2
    } else {
        1
    };
    // Block 0 has to state the tune, and the last block is the final
    // chorus, which forces every layer back on anyway.
    let slots: Vec<usize> = (1..blocks - 1).collect();
    // The breakdown belongs in the second half: it is the air before the
    // last chorus, not a hole near the top of the piece. It stays clear
    // of the second-to-last block, whose last bar is the run-up fill into
    // the final chorus — that bar is loud by design, and a block that
    // drops everything and then plays the biggest fill in the piece is
    // not a breakdown, it is a break that changed its mind.
    let late: Vec<usize> = slots
        .iter()
        .copied()
        .filter(|&i| i * 2 >= blocks && i + 2 < blocks)
        .collect();
    for _ in 0..breaks {
        place_role(BlockRole::Break, &late, &mut roles, rng);
    }
    for _ in 0..chords {
        place_role(BlockRole::Chords, &slots, &mut roles, rng);
    }
    roles
}

struct Material {
    sections: Vec<Section>,
    form: Vec<usize>,
    roles: Vec<BlockRole>,
    /// True when the progression is [`MARUSA`], which pins the mode — see
    /// [`MARUSA_MODE`].
    marusa: bool,
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
    /// The dance kit: a saturated sub kick and a clap on both backbeats,
    /// with the chip hats still on top. Rolled by the hard-bass groove,
    /// but it is a kit like any other and plays whatever pattern the
    /// composer wrote.
    Hard,
}

impl Kit {
    fn kick(self) -> Inst {
        match self {
            Kit::Chip => Inst::Kick,
            Kit::Sampled => Inst::PcmKick,
            Kit::Hard => Inst::HardKick,
        }
    }
    fn snare(self, clap: bool) -> Inst {
        match (self, clap) {
            (Kit::Chip, _) => Inst::Snare,
            (Kit::Sampled, false) => Inst::PcmSnare,
            (Kit::Sampled, true) => Inst::Clap,
            // Both backbeats clap: on a four-on-the-floor bar a snare
            // reads as a rock band sitting in, which is the one thing
            // this kit is not.
            (Kit::Hard, _) => Inst::Clap,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kit::Chip => "chip",
            Kit::Sampled => "pcm",
            Kit::Hard => "hard",
        }
    }

    pub const ALL: [Kit; 3] = [Kit::Chip, Kit::Sampled, Kit::Hard];
}

/// What a piece is built around rhythmically.
///
/// This is a bigger switch than the kit: it decides the bass figure, what
/// the third pulse plays, where the kick sits, whether the mix pumps under
/// it, and — where the profile's mode allows it — the progression. One
/// roll per piece, so a session hears both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Groove {
    /// Whatever the profile's own arrangement says. The house style.
    Normal,
    /// Hard bass: a kick on every beat with the whole mix ducking out of
    /// its way, the bass answering on the offbeats under a sawtooth donk
    /// an octave above it, chord stabs between, and release-cut piano
    /// scattered over the sixteenths. Nothing here is a new idea — it is
    /// what a dance record does — but on nine chip voices it is the
    /// loudest a piece can be without playing more notes.
    HardBass,
}

impl Groove {
    pub fn name(self) -> &'static str {
        match self {
            Groove::Normal => "normal",
            Groove::HardBass => "hard bass",
        }
    }

    pub const ALL: [Groove; 2] = [Groove::Normal, Groove::HardBass];
}

/// How often each profile reaches for the hard-bass groove.
///
/// Only where a beat is the point. Zen has no business pumping, the menus
/// have no drums to pump with, and the fanfare is over before the groove
/// would establish itself.
fn hard_bass_chance(profile: Profile) -> f32 {
    match profile {
        Profile::VsIntense => 0.65,
        Profile::ZenPeak => 0.50,
        Profile::SoloCalm => 0.35,
        Profile::Zen | Profile::Ambient | Profile::Victory => 0.0,
    }
}

/// How often a piece reaches for the maru-sa changes on its own, without
/// the groove asking for them.
///
/// Zero wherever the mode they need would be wrong for the profile — see
/// [`MARUSA_MODE`]. Those are also exactly the profiles whose whole
/// character is the mode, so there is nothing to regret.
fn marusa_chance(profile: Profile) -> f32 {
    match profile {
        Profile::VsIntense => 0.45,
        Profile::SoloCalm => 0.30,
        Profile::Ambient => 0.15,
        Profile::Zen | Profile::ZenPeak | Profile::Victory => 0.0,
    }
}

/// True where plain minor is a mode the profile would have chosen anyway,
/// which is the condition for the maru-sa changes being available at all.
fn marusa_fits(profile: Profile) -> bool {
    marusa_chance(profile) > 0.0
}

fn roll_groove(profile: Profile, rng: &mut Rng) -> Groove {
    if rng.chance(hard_bass_chance(profile)) {
        Groove::HardBass
    } else {
        Groove::Normal
    }
}

impl Profile {
    pub const ALL: [Profile; 6] = [
        Profile::Ambient,
        Profile::SoloCalm,
        Profile::VsIntense,
        Profile::Victory,
        Profile::Zen,
        Profile::ZenPeak,
    ];

    /// Short English name, for the listening screen.
    pub fn name(self) -> &'static str {
        match self {
            Profile::Ambient => "AMBIENT",
            Profile::SoloCalm => "SOLO",
            Profile::VsIntense => "VERSUS",
            Profile::Victory => "VICTORY",
            Profile::Zen => "ZEN",
            Profile::ZenPeak => "ZEN PEAK",
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
        // At the peak the pulse is the point, and a pulse wants four.
        Profile::ZenPeak => [0.70, 0.10, 0.20],
    };
    [Meter::Four, Meter::Three, Meter::Six][rng.weighted(&w)]
}

/// Antecedent/consequent plans: which motif each bar of a section sings,
/// and what is done to it.
///
/// Every plan starts on motif 0 and brings it back at least twice more,
/// which is the cheapest possible guarantee of audible repetition; every
/// plan sequences a motif up a third somewhere in the second half, which
/// is where the phrase peaks; and every plan closes on a cadence.
/// Everything else is free, and each section of a piece draws its own —
/// so B does not merely put new chords under the order A already sang.
const PLANS: [[(usize, Transform); 8]; 5] = [
    [
        (0, Transform::None),
        (1, Transform::None),
        (0, Transform::None),
        (2, Transform::None),
        (0, Transform::None),
        (1, Transform::Up),
        (3, Transform::None),
        (2, Transform::Cadence),
    ],
    [
        (0, Transform::None),
        (0, Transform::None),
        (1, Transform::None),
        (2, Transform::None),
        (0, Transform::None),
        (0, Transform::Up),
        (1, Transform::None),
        (3, Transform::Cadence),
    ],
    [
        (0, Transform::None),
        (1, Transform::None),
        (2, Transform::None),
        (0, Transform::None),
        (0, Transform::None),
        (1, Transform::Up),
        (2, Transform::None),
        (1, Transform::Cadence),
    ],
    [
        (0, Transform::None),
        (1, Transform::None),
        (0, Transform::None),
        (1, Transform::None),
        (2, Transform::None),
        (0, Transform::Up),
        (3, Transform::None),
        (1, Transform::Cadence),
    ],
    [
        (0, Transform::None),
        (2, Transform::None),
        (0, Transform::None),
        (3, Transform::None),
        (0, Transform::None),
        (2, Transform::Up),
        (1, Transform::None),
        (3, Transform::Cadence),
    ],
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
        // Still a zen melody, just one with somewhere to be.
        Profile::ZenPeak => 0.70,
        // The two that are allowed some angularity: a fanfare wants
        // fanfare intervals, and versus wants energy.
        Profile::Victory => 0.50,
        Profile::VsIntense => 0.50,
    }
}

fn build_material(
    seed: u64,
    profile: Profile,
    meter: Meter,
    smoothness: f32,
    groove: Groove,
) -> Material {
    let mut rng = Rng::new(seed ^ ((profile as u64 + 1) * 0x9E37_79B9));
    // Everything but the fanfare lives in minor. Major progressions read
    // as cheerful, and cheerful is the wrong register for a stack that is
    // about to top out.
    let dark = !matches!(profile, Profile::Victory);
    let bank = rhythm_bank(meter);

    // Four motifs, each on its own rhythm cell, drawn evenly.
    //
    // Versus used to draw from a window of the busiest cells instead, to
    // make it denser. It worked and it was wrong: a fast profile with a
    // relentless tune has no room left to build, and the melody is the
    // one part a player is following while they are trying to think. The
    // contrast belongs in the form, not in the note count.
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

    // The groove can call for the maru-sa changes, and a piece can reach
    // for them on its own. The roll happens either way so that pinning a
    // groove does not shift every draw after this one.
    let rolled_marusa = rng.chance(marusa_chance(profile));
    let marusa = marusa_fits(profile) && (rolled_marusa || groove == Groove::HardBass);

    let loops: &[[i32; 4]] = if dark { &DARK_LOOPS } else { &CALM_LOOPS };
    let mut sections = Vec::new();
    for _ in 0..3 {
        let base = *rng.pick(loops);
        // The consequent phrase may take a second progression. Four bars
        // stated twice under two different cadences is the plainest thing
        // eight bars of harmony can be, and the bank is right there.
        let answer = if rng.chance(0.45) {
            *rng.pick(loops)
        } else {
            base
        };
        let mut chords = [Chord::new(0); 8];
        for (i, c) in chords.iter_mut().enumerate() {
            // The maru-sa loop is the whole eight bars, twice — it is a
            // cycle rather than a phrase, and taking an answering
            // progression to it would only break the cycle.
            *c = if marusa {
                MARUSA[i % 4]
            } else {
                Chord::new(if i < 4 { base[i] } else { answer[i % 4] })
            };
        }
        // Scheduled cadences: half at the end of the antecedent phrase,
        // authentic at the end of the consequent.
        chords[3] = Chord::new(4);
        chords[7] = Chord::new(0);
        // A seventh on the dominant sharpens the pull home. This one is
        // structural rather than colour, so every profile gets it.
        chords[3].ext = Ext::Seventh;
        if marusa {
            // Where the changes are the point, the half cadence is the
            // real thing: a major third and a flat seventh, not the minor
            // seventh chord the mode would otherwise put on this degree.
            // It is the chord the rotation was arranged around.
            chords[3] = MARUSA[3];
        } else {
            // Colour is for progressions that were not written with their
            // own; sprinkling sevenths onto these would undo them.
            extend(&mut chords, profile, &mut rng);
        }

        // Each section sings its own plan, and each bar knows what the
        // one before it ended on so the line can carry over the bar line.
        let plan = rng.pick(&PLANS);
        let mut melody: Vec<Vec<MelNote>> = Vec::with_capacity(8);
        for bar in 0..8 {
            let (mi, tf) = plan[bar];
            let prev = melody
                .last()
                .and_then(|b: &Vec<MelNote>| b.last())
                .map(|n| n.deg);
            melody.push(realize(&motifs[mi], tf, chords[bar], meter, prev));
        }

        sections.push(Section {
            chords,
            melody,
            hat_rot: rng.below(4),
            kick_rot: 0,
        });
    }

    // Three sections of material carry the whole form, with section A
    // returning more often than not — under 30% new material, which is
    // where real songs sit.
    let form = rng.pick(&FORMS).to_vec();
    let roles = roll_roles(profile, form.len(), &mut rng);

    Material {
        sections,
        form,
        roles,
        marusa,
        rhythms_used,
    }
}

/// Stack thirds onto the progression, by profile.
///
/// Extensions are the cheapest richness available: nothing plays more
/// notes at once (the hardware could not), but the arpeggio macro walks
/// a wider chord and the melody is allowed to land on the seventh and
/// the ninth, so the same arrangement stops sounding like three notes
/// repeated. Versus gets the most of it because that is where a thin
/// progression reads as cheap rather than as restraint; the calm
/// profiles get a light touch, because a Dorian pad wants air more than
/// it wants a ninth.
fn extend(chords: &mut [Chord; 8], profile: Profile, rng: &mut Rng) {
    let (seventh, ninth) = match profile {
        Profile::VsIntense => (0.75, 0.40),
        Profile::Victory => (0.60, 0.35),
        Profile::SoloCalm => (0.30, 0.10),
        Profile::Zen => (0.25, 0.12),
        Profile::Ambient => (0.20, 0.08),
        // Bright modes plus sevenths and ninths is the dance-record
        // sound this one is after.
        Profile::ZenPeak => (0.60, 0.35),
    };
    for (i, c) in chords.iter_mut().enumerate() {
        // The two cadence bars are load-bearing: bar 3 keeps its dominant
        // seventh, and bar 7 is home, which wants to sound like an
        // arrival rather than like one more colour chord.
        if i == 3 || i == 7 {
            continue;
        }
        if rng.chance(seventh) {
            c.ext = if rng.chance(ninth) {
                Ext::Ninth
            } else {
                Ext::Seventh
            };
        }
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
    // Smoothness moves weight out of the wide intervals and into seconds:
    // seconds 0.60 -> 0.88, thirds 0.32 -> 0.12, fourths and fifths
    // 0.08 -> 0.
    //
    // The repeat is not a fifth wheel. Every corpus anyone has counted
    // has a repeated note as one of the commonest melodic "intervals",
    // and a walk forbidden from staying put is the single clearest tell
    // of a generated line: it has to be going somewhere on every onset,
    // so it never lets a note simply land.
    let same = 0.15;
    let second = 0.30 + 0.14 * s;
    let third = 0.16 - 0.10 * s;
    let fourth = 0.02 - 0.02 * s;
    let fifth = 0.02 - 0.02 * s;
    let weights = [
        same, second, second, third, third, fourth, fourth, fifth, fifth,
    ];

    let mut out = Vec::with_capacity(n);
    let mut d = *rng.pick(&[0, 2, 4]);
    // The last move that actually went somewhere, so a leap can be
    // answered even if a repeated note comes between.
    let mut last_move = 0i32;
    for _ in 0..n {
        out.push(d);
        let mut step: i32 = match rng.weighted(&weights) {
            0 => 0,
            1 => -1,
            2 => 1,
            3 => -2,
            4 => 2,
            5 => -3,
            6 => 3,
            7 => -4,
            _ => 4,
        };
        // Post-skip reversal: a leap is answered by a step back the other
        // way. Of all the melodic regularities that have been measured
        // this is the strongest, and it is what makes a leap sound like
        // a gesture with a consequence rather than like the line coming
        // apart. Only leaps ask for it — a run of seconds is already a
        // line and should be allowed to keep going.
        if last_move.abs() >= 2 && step != 0 && rng.chance(0.55 + 0.25 * s) {
            step = -last_move.signum();
        }
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
        if step != 0 {
            last_move = step;
        }
        d = (d + step).clamp(CONTOUR_LOW, CONTOUR_HIGH);
    }
    out
}

/// How far apart two notes either side of a bar line may sit before the
/// seam stops sounding like a melody continuing and starts sounding like
/// two melodies taking turns. A fifth is the widest interval the contour
/// itself will write, so anything past it can only have come from the
/// join.
const SEAM_MAX: i32 = 4;

/// What the join is allowed to transpose a bar by: a third or a fifth,
/// either direction. Both are sequences, which is a thing melodies do on
/// purpose; anything else would be a different melody.
const SEAM_SHIFTS: [i32; 5] = [0, -2, 2, -4, 4];

/// How far outside the contour's own register a linked bar may sit. This
/// is what stops the join from chasing: without it a bar that ended high
/// pulls the next one up, that one ends higher still, and eight bars
/// later the tune is shrieking.
const LINK_SLACK: i32 = 2;

/// The nearest chord tone strictly `dir`-wards of `from`, as long as it
/// stays inside the melodic register. `None` when there is nowhere to go,
/// in which case the note it was called for keeps the pitch it had.
fn next_chord_tone(chord: Chord, from: i32, dir: i32) -> Option<i32> {
    (1..=4)
        .map(|k| from + dir * k)
        .filter(|c| (CONTOUR_LOW - LINK_SLACK..=CONTOUR_HIGH + LINK_SLACK).contains(c))
        .find(|c| chord.contains(*c))
}

/// Turn a motif into the actual notes of one bar over `chord`.
///
/// `prev_last` is the degree the bar before this one ended on, if there
/// was one.
fn realize(
    motif: &Motif,
    tf: Transform,
    chord: Chord,
    meter: Meter,
    prev_last: Option<i32>,
) -> Vec<MelNote> {
    let rhythm = rhythm_bank(meter)[motif.rhythm];
    let plan_shift = match tf {
        Transform::None | Transform::Cadence => 0,
        Transform::Up => 2,
    };
    // Voice-lead across the bar line. A motif starts wherever its own
    // contour happens to start, so a bar that ended high followed by one
    // that starts low used to drop an octave in a single move — inside
    // the bars the walk is careful, and every eight of those care was
    // thrown away at the join.
    //
    // The fix has to keep the motif recognizable, so the only thing
    // allowed to change is the register it is sung in — exactly the
    // transposition `Transform::Up` already applies, which is to say a
    // sequence. Nothing moves unless the seam is genuinely too wide, and
    // nothing moves far enough to leave the melodic register.
    let link = match prev_last {
        Some(prev) if (motif.contour[0] + plan_shift - prev).abs() > SEAM_MAX => {
            let first = motif.contour[0] + plan_shift;
            let lo = motif.contour.iter().min().copied().unwrap_or(0) + plan_shift;
            let hi = motif.contour.iter().max().copied().unwrap_or(0) + plan_shift;
            SEAM_SHIFTS
                .into_iter()
                .filter(|s| lo + s >= CONTOUR_LOW - LINK_SLACK)
                .filter(|s| hi + s <= CONTOUR_HIGH + LINK_SLACK)
                .min_by_key(|s| ((first + s - prev).abs(), s.abs()))
                .unwrap_or(0)
        }
        _ => 0,
    };
    let shift = plan_shift + link;
    let mut out = Vec::with_capacity(rhythm.len());
    let mut prev: Option<(i32, i32)> = None;
    for (i, &(step, len)) in rhythm.iter().enumerate() {
        let raw = motif.contour[i.min(motif.contour.len() - 1)] + shift;
        let mut deg = raw;
        let prio = meter.prio(step);
        // Strong beats must be chord tones: this single rule is the
        // difference between "a melody over the changes" and "notes".
        if prio == 0 {
            deg = chord.snap(raw);
            // Snapping is also what can collapse a line onto one note.
            // Several of the rhythm cells put every onset on a beat, so
            // every note in the bar is snapped, and a narrow walk over
            // one of those comes out as the same chord tone struck four
            // times — the tune stops being a line and starts being a
            // pulse. Where the walk did mean to move, let it: to the
            // next chord tone along, in the direction it was going.
            if let Some((p_deg, p_raw)) = prev
                && deg == p_deg
                && raw != p_raw
            {
                let dir = if raw > p_raw { 1 } else { -1 };
                deg = next_chord_tone(chord, deg, dir).unwrap_or(deg);
            }
        }
        prev = Some((deg, raw));
        out.push(MelNote {
            step,
            len,
            deg,
            prio,
        });
    }
    // Fill a third stepwise. Snapping the strong beats to the chord can
    // leave a weak note sitting outside the two notes it comes between,
    // so a line that wanted to walk up a third jumps past its own
    // destination and comes back. Where the outer two are a third apart
    // there is exactly one note in the middle, and putting the weak note
    // there is what a passing tone is.
    for i in 1..out.len().saturating_sub(1) {
        if out[i].prio == 0 {
            continue;
        }
        let (a, b) = (out[i - 1].deg, out[i + 1].deg);
        let between = (out[i].deg - a).signum() == (b - out[i].deg).signum();
        if (b - a).abs() == 2 && !between {
            out[i].deg = a + (b - a).signum();
        }
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

/// Where the hard-bass groove keeps the pulse whatever the profile
/// usually does. Below this it stops bouncing; above it the kick stops
/// having room to be a kick, and the whole thing turns into speedcore.
const HARD_BASS_BPM: (f32, f32) = (148.0, 176.0);

/// Tempo is rolled **once per piece and never moves again.**
///
/// It used to climb with the danger level, NES-Tetris style. That is a
/// real convention and it does raise the stakes, but a tempo that shifts
/// under you is genuinely hard to play along with, and a puzzle game is
/// something you play *along with* for half an hour at a stretch. The
/// intensity signal still has plenty of voice: mode, note density, drum
/// density, bass figure, duty cycle and which layers are audible all
/// keep moving. Only the pulse holds still.
///
/// The groove has a say because a groove is a tempo as much as it is a
/// pattern: four on the floor at 200 BPM is not hard bass, it is
/// something else with the same drum pattern.
fn tempo_range(profile: Profile, groove: Groove) -> (f32, f32) {
    let (lo, hi) = profile_tempo_range(profile);
    if groove != Groove::HardBass {
        return (lo, hi);
    }
    // Keep whatever the profile and the groove already agree on; where
    // they do not overlap at all, the groove wins. A versus piece is
    // otherwise faster than this bounces and a solo piece is slower.
    let (floor, ceiling) = HARD_BASS_BPM;
    let (lo, hi) = (lo.max(floor), hi.min(ceiling));
    if lo < hi { (lo, hi) } else { HARD_BASS_BPM }
}

/// The window a profile picks from when nothing else has an opinion.
fn profile_tempo_range(profile: Profile) -> (f32, f32) {
    match profile {
        // Slower than the menus, which is to say slower than anything
        // else the composer writes.
        Profile::Zen => (74.0, 88.0),
        Profile::Ambient => (92.0, 104.0),
        Profile::Victory => (128.0, 138.0),
        Profile::SoloCalm => (122.0, 144.0),
        // Roughly double zen's pulse, which is what makes the takeover
        // read as the same mode shifting gear rather than a new one.
        Profile::ZenPeak => (150.0, 168.0),
        Profile::VsIntense => (178.0, 206.0),
    }
}

/// Rolled per piece. Sampled drums are the exception rather than the
/// house style: they are a change of texture, and the chip kit is what
/// makes the rest of the arrangement sound like a chip.
fn roll_kit(profile: Profile, groove: Groove, rng: &mut Rng) -> Kit {
    let sampled = match profile {
        Profile::Zen => 0.20,
        Profile::Ambient => 0.25,
        Profile::SoloCalm => 0.35,
        Profile::ZenPeak => 0.45,
        Profile::VsIntense => 0.45,
        Profile::Victory => 0.5,
    };
    // Drawn either way, so pinning the groove does not shift the piece's
    // remaining rolls.
    let sampled = rng.chance(sampled);
    if groove == Groove::HardBass {
        return Kit::Hard;
    }
    if sampled { Kit::Sampled } else { Kit::Chip }
}

fn roll_tempo(profile: Profile, meter: Meter, groove: Groove, rng: &mut Rng) -> f32 {
    let (lo, hi) = tempo_range(profile, groove);
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
/// The scales a profile may use at each intensity band.
///
/// Each band offers a choice rather than one answer: a piece picks a
/// lane at the start and stays in it, so within one tune the ladder is
/// still a ladder, but two pieces at the same danger level do not have
/// to be in the same scale. One mode per band made every versus round
/// sound like the last one.
///
/// Versus is plain minor throughout. It used to end on Phrygian, and a
/// flat second is a very particular flavour — Spanish, not tense — which
/// is not what a match wants at its hottest. Harmonic and melodic minor
/// do the same job by raising the seventh, which pulls rather than
/// colours.
fn mode_palette(profile: Profile, band: usize) -> &'static [Mode] {
    match profile {
        // Menus stay put: one mood, no ladder to climb.
        Profile::Ambient => &[Mode::Dorian, Mode::Aeolian],
        Profile::Victory => &[Mode::Lydian, Mode::Ionian, Mode::Mixolydian],
        // Zen inverts the usual reading of intensity. Nothing here is at
        // stake, so a clear field gets the brightest modes in the set and
        // a tall stack only shades them back toward minor — colour, not
        // a warning.
        Profile::Zen => match band {
            0 => &[Mode::Lydian, Mode::Ionian],
            1 => &[Mode::Ionian, Mode::Mixolydian],
            2 => &[Mode::Mixolydian, Mode::Dorian],
            _ => &[Mode::Dorian, Mode::Aeolian],
        },
        // The peak keeps zen's inverted ladder — there is still nothing
        // to lose — but leans on the two modes with a flat seventh,
        // which is what a fast bright piece wants under it.
        Profile::ZenPeak => match band {
            0 => &[Mode::Lydian, Mode::Ionian],
            1 => &[Mode::Mixolydian, Mode::Ionian],
            2 => &[Mode::Mixolydian, Mode::Dorian],
            _ => &[Mode::Dorian, Mode::MelodicMinor],
        },
        Profile::SoloCalm => match band {
            0 => &[Mode::Dorian, Mode::Aeolian],
            1 => &[Mode::Aeolian, Mode::Dorian],
            2 => &[Mode::Aeolian, Mode::MelodicMinor],
            _ => &[Mode::HarmonicMinor, Mode::Aeolian, Mode::MelodicMinor],
        },
        Profile::VsIntense => match band {
            0 => &[Mode::Aeolian, Mode::Dorian],
            1 => &[Mode::Aeolian, Mode::Dorian, Mode::MelodicMinor],
            2 => &[Mode::Aeolian, Mode::MelodicMinor, Mode::HarmonicMinor],
            _ => &[Mode::HarmonicMinor, Mode::MelodicMinor, Mode::Aeolian],
        },
    }
}

/// `variant` is the lane this piece rolled; it wraps, so a palette may
/// be any length.
fn mode_target(profile: Profile, intensity: f32, variant: usize) -> Mode {
    let palette = mode_palette(profile, band(intensity));
    palette[variant % palette.len()]
}

/// What the third pulse channel does when it is switched on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Counter {
    /// A quiet delayed copy of the melody, panned opposite it.
    Echo,
    /// A fast chord arpeggio in sixteenths.
    Arp,
    /// Release-cut piano: chord tones scattered over the sixteenths, each
    /// one cut before it can ring. The same notes an arpeggio would play,
    /// with holes in the pattern and no sustain — which turns the third
    /// pulse from a texture into part of the rhythm section.
    CutPiano,
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
    /// How often the chord voice restrikes. Expressed against the beat
    /// rather than as a count per bar so 3/4 and 6/8 come out right
    /// without a divisor table.
    harm_rate: HarmRate,
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

/// How often the chord voice restrikes within a bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HarmRate {
    /// Twice a bar — a chord bed that sits still.
    HalfBar,
    /// Once per beat.
    Beat,
    /// Twice per beat. Dense enough to be part of the rhythm section
    /// rather than the background.
    Eighth,
}

impl HarmRate {
    /// Steps between hits, and how many fit in the bar.
    fn grid(self, meter: Meter) -> (u32, u32) {
        let spb = meter.steps_per_beat();
        let span = match self {
            HarmRate::HalfBar => meter.steps() / 2,
            HarmRate::Beat => spb,
            HarmRate::Eighth => (spb / 2).max(1),
        };
        (span, meter.steps() / span)
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
        // Zen's mallets and strings, plus the one attack that can carry
        // a tune at this tempo.
        Profile::ZenPeak => &[
            Inst::Pluck,
            Inst::Marimba,
            Inst::Guitar,
            Inst::Piano,
            Inst::Sustain,
        ],
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

/// The bass figure the hard-bass groove plays: see [`bass_pattern`].
const BASS_OFFBEAT: usize = 5;

fn arrange(profile: Profile, ctx: &Context, lead: Inst, groove: Groove) -> Arrangement {
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
        // This one takes over a run already in progress, at the exact
        // moment the player notices the floor move. Anything held back
        // would read as the music having stopped, not started.
        Profile::ZenPeak => (0.0, 0.0, 2.0),
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
            harm_rate: HarmRate::HalfBar,
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
            harm_rate: HarmRate::HalfBar,
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
        // Zen's instruments over a dance floor: the pad and the mallets
        // stay, the drums stop being a texture and become a beat.
        Profile::ZenPeak => Arrangement {
            lead: t >= l_at,
            harmony: t >= h_at,
            perc: t >= p_at,
            pad: Some(if i < 0.6 {
                Inst::Glass
            } else {
                Inst::WaveOrgan
            }),
            counter: (t >= l_at + 8.0).then_some(Counter::Arp),
            saw: (i >= 0.55).then_some(SawRole::LeadDouble),
            shaker: i >= 0.40,
            harm_rate: if i < 0.5 {
                HarmRate::Beat
            } else {
                HarmRate::Eighth
            },
            lead_inst: lead,
            harm_inst: if i < 0.6 { Inst::Pad } else { Inst::Organ },
            bass_pat: [1, 2, 2, 3][b],
            hat_k: [4, 8, 8, 11][b],
            kick_k: 4,
            four_floor: true,
            kick_extra: [&[][..], &[14], &[7, 14], &[7, 14]][b],
            snare: true,
            swing: 0.52,
            max_prio: 1,
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
            harm_rate: if i < 0.55 {
                HarmRate::HalfBar
            } else {
                HarmRate::Beat
            },
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
            harm_rate: [
                HarmRate::Beat,
                HarmRate::Beat,
                HarmRate::Eighth,
                HarmRate::Eighth,
            ][b],
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
            harm_rate: HarmRate::Beat,
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

    // The hard-bass treatment. Everything above the low end gets out of
    // the kick's way and comes back in the gaps: the kick takes every
    // beat, the bass answers between the beats instead of under them
    // (the kick's own body is the note on the beat), the chords stab
    // where the bass is not, and the piano fills the sixteenths that are
    // left. The pumping is in `Synth::set_pump`, which is the other half
    // of this and the half you feel rather than hear.
    if groove == Groove::HardBass {
        a.perc = true;
        a.four_floor = true;
        a.kick_extra = [&[][..], &[14], &[7, 14], &[7, 11, 14]][b];
        a.snare = true;
        // Nothing about this swings. A shuffled sixteenth under a
        // four-on-the-floor kick is a different genre entirely.
        a.swing = 0.5;
        a.bass_pat = BASS_OFFBEAT;
        // The saw doubles that offbeat an octave up: the donk.
        a.saw = Some(SawRole::BassDouble);
        a.harm_inst = Inst::Stab;
        a.harm_rate = HarmRate::Eighth;
        a.hat_k = [6, 8, 11, 13][b];
        a.shaker = i >= 0.35;
        // The piano arrives with the melody rather than a quarter of a
        // minute later: it is a hook here, not an ornament.
        a.counter = (t >= l_at).then_some(Counter::CutPiano);
        a.max_prio = a.max_prio.max(1);
    }

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
    pub groove: Groove,
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
            "{} {}・{}・{:.0} BPM・{}{}・#{:06x}",
            NOTE_NAMES[(self.tonic.rem_euclid(12)) as usize],
            self.mode.name(),
            self.meter.name(),
            self.bpm,
            lead_name(self.lead),
            // The house groove goes unmentioned; there is no room in a
            // toast for a field that reads the same every time.
            match self.groove {
                Groove::Normal => "",
                Groove::HardBass => "・hard",
            },
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
    /// What this piece is built around rhythmically. Rolled per piece,
    /// and the one switch that reaches the synthesizer as well as the
    /// score — see [`Composer::pumps`].
    groove: Groove,
    /// Set by [`Composer::force_groove`]; survives re-rolls.
    groove_override: Option<Groove>,
    /// The melody instrument the last planned bar actually used. It is
    /// one of the two below, and which one depends on where in the form
    /// that bar sat.
    lead: Inst,
    /// Set by [`Composer::force_lead`]; survives re-rolls.
    lead_override: Option<Inst>,
    /// The pair this piece rolled: the first half of the form, and the
    /// one the melody switches to for the second half.
    lead_first: Inst,
    lead_alt: Inst,
    /// Which lane of each intensity band's scale palette this piece
    /// uses. Rolled per piece, so the ladder stays a ladder within a
    /// tune while two tunes need not share a scale.
    mode_variant: usize,
    /// How this piece plays its chord blocks, and whether the strum
    /// pattern is the busy one. Rolled per piece like the kit.
    chord_style: ChordStyle,
    strum_busy: bool,
    style_override: Option<ChordStyle>,
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
            material: build_material(
                seed,
                profile,
                Meter::Four,
                default_smoothness(profile),
                Groove::Normal,
            ),
            mode: mode_target(profile, 0.0, 0),
            root,
            bpm: roll_tempo(profile, Meter::Four, Groove::Normal, &mut Rng::new(seed)),
            meter: Meter::Four,
            kit: Kit::Chip,
            groove: Groove::Normal,
            groove_override: None,
            lead: lead_palette(profile)[0],
            lead_override: None,
            lead_first: lead_palette(profile)[0],
            lead_alt: lead_palette(profile)[0],
            mode_variant: 0,
            chord_style: ChordStyle::Sustain,
            strum_busy: false,
            style_override: None,
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
            groove: self.groove,
            lead: self.lead_first,
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
        // Rolled before the tempo and the kit because it decides both.
        let groove = roll_groove(profile, &mut rng);
        self.groove = self.groove_override.unwrap_or(groove);
        self.bpm = roll_tempo(profile, self.meter, self.groove, &mut rng);
        self.kit = roll_kit(profile, self.groove, &mut rng);
        self.smoothness = self
            .smooth_override
            .unwrap_or_else(|| default_smoothness(profile));
        let palette = lead_palette(profile);
        // An explicit pin wins over the palette — the palettes are taste,
        // and someone auditioning an instrument wants to hear it on
        // whatever preset they picked. It still has to belong to the lead
        // channel, or it would silently land on someone else's voice.
        self.lead_first = self
            .lead_override
            .filter(|i| i.is_melody())
            .unwrap_or_else(|| palette[rng.below(palette.len())]);
        self.lead_alt = self.roll_alt_lead(&mut rng, palette);
        self.lead = self.lead_first;
        self.chord_style = self.style_override.unwrap_or_else(|| {
            if rng.chance(strum_chance(profile)) {
                ChordStyle::Strum
            } else {
                ChordStyle::Sustain
            }
        });
        // A busy strum under a fast profile is a lot of hand; under a
        // slow one it is what makes the block move at all.
        self.strum_busy = rng.chance(match profile {
            Profile::Zen => 0.35,
            Profile::Ambient => 0.45,
            _ => 0.7,
        });
        // Rolled last, so the draws `force_lead` replays to recover this
        // piece's melody instrument stay exactly where they were.
        self.mode_variant = rng.below(6);
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            profile,
            self.meter,
            self.smoothness,
            self.groove,
        );
        self.mode = self.piece_mode(intensity);
        self.bar = 0;
        // A new song starts back in its own key.
        self.lift = 0;
    }

    /// The scale this piece plays in at `intensity`.
    ///
    /// Normally the intensity ladder decides, one mode per band. A piece
    /// on the maru-sa changes does not get a vote: those chords only
    /// spell themselves in one mode — see [`MARUSA_MODE`].
    fn piece_mode(&self, intensity: f32) -> Mode {
        if self.material.marusa {
            return MARUSA_MODE;
        }
        mode_target(self.profile, intensity, self.mode_variant)
    }

    /// Override the drum kit [`Composer::set_profile`] rolled. Used by
    /// the offline renderer to audition a kit on demand.
    pub fn force_kit(&mut self, kit: Kit) {
        self.kit = kit;
    }

    /// Override the groove [`Composer::set_profile`] rolled, rebuilding
    /// the piece around it: the groove decides the tempo window, the kit
    /// and — where the mode allows it — the progression, so none of those
    /// can be left as they were. `None` hands the choice back to the roll.
    pub fn force_groove(&mut self, groove: Option<Groove>) {
        if groove == self.groove_override {
            return;
        }
        self.groove_override = groove;
        // What this piece's own roll would have chosen, by replaying the
        // draws `set_profile` made up to that point.
        let mut rng = Rng::new(self.seed ^ (self.piece.wrapping_mul(0x1000_0193)));
        roll_meter(self.profile, &mut rng);
        let rolled = roll_groove(self.profile, &mut rng);
        let want = groove.unwrap_or(rolled);
        if want == self.groove {
            return;
        }
        self.groove = want;
        self.bpm = roll_tempo(self.profile, self.meter, want, &mut rng);
        self.kit = roll_kit(self.profile, want, &mut rng);
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            self.profile,
            self.meter,
            self.smoothness,
            want,
        );
        self.bar = 0;
    }

    pub fn groove(&self) -> Groove {
        self.groove
    }

    /// Whether the kick should duck the rest of the mix. The one thing
    /// the composer has to say to the synthesizer that is not a note.
    pub fn pumps(&self) -> bool {
        self.groove == Groove::HardBass
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
            self.groove,
            &mut Rng::new(self.seed ^ self.piece.wrapping_mul(0x1000_0193)),
        );
        self.material = build_material(
            self.seed ^ self.piece.wrapping_mul(0xA24B_AED4),
            self.profile,
            meter,
            self.smoothness,
            self.groove,
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
            self.groove,
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
        let lead = lead.filter(|i| i.is_melody());
        if lead == self.lead_override {
            return;
        }
        self.lead_override = lead;
        let palette = lead_palette(self.profile);
        match lead {
            // A pin holds for the whole form: no mid-piece switch either,
            // or auditioning one instrument would still play the other.
            Some(pinned) => {
                self.lead_first = pinned;
                self.lead_alt = pinned;
            }
            None => {
                // Back to what this piece's own roll would have chosen,
                // by replaying the draws `set_profile` made.
                let mut rng = Rng::new(self.seed ^ (self.piece.wrapping_mul(0x1000_0193)));
                roll_meter(self.profile, &mut rng);
                roll_groove(self.profile, &mut rng);
                roll_tempo(self.profile, self.meter, self.groove, &mut rng);
                roll_kit(self.profile, self.groove, &mut rng);
                self.lead_first = palette[rng.below(palette.len())];
                self.lead_alt = self.roll_alt_lead(&mut rng, palette);
            }
        }
        self.lead = self.lead_first;
    }

    /// A different instrument from `self.lead_first`, for the second half of
    /// the form. A pinned lead pins both — someone auditioning one
    /// instrument does not want the other one turning up.
    fn roll_alt_lead(&self, rng: &mut Rng, palette: &[Inst]) -> Inst {
        if self.lead_override.is_some() || palette.len() < 2 {
            return self.lead_first;
        }
        for _ in 0..8 {
            let pick = palette[rng.below(palette.len())];
            if pick != self.lead_first {
                return pick;
            }
        }
        // Vanishingly unlikely; take whatever is not the first entry.
        *palette
            .iter()
            .find(|i| **i != self.lead_first)
            .unwrap_or(&self.lead_first)
    }

    /// The instrument the melody is on right now. This moves partway
    /// through the form, so it is what the readouts should show.
    pub fn lead(&self) -> Inst {
        self.lead
    }

    /// Override how the chord blocks play their chord. `None` hands the
    /// choice back to the per-piece roll.
    pub fn force_chord_style(&mut self, style: Option<ChordStyle>) {
        self.style_override = style;
        if let Some(style) = style {
            self.chord_style = style;
        }
    }

    pub fn chord_style(&self) -> ChordStyle {
        self.chord_style
    }

    /// The instrument this piece rolled for the first half of the form.
    pub fn lead_first(&self) -> Inst {
        self.lead_first
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
        let phrase_start = self.bar.is_multiple_of(BARS_PER_SECTION);
        if phrase_start {
            // Mode changes only at phrase boundaries; mid-bar they sound
            // like a bug rather than a modulation.
            self.mode = self.piece_mode(ctx.intensity);
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

        // The melody changes instrument halfway through the form, so a
        // long piece does not spend eighty bars on one timbre. One swap,
        // at the midpoint — alternating every block or two reads as a
        // fault rather than as an arrangement, and it puts the second
        // instrument on the final chorus, where a new colour pays off.
        let role = self.material.roles[block];
        self.lead = if block >= blocks / 2 {
            self.lead_alt
        } else {
            self.lead_first
        };

        let mut arr = arrange(self.profile, ctx, self.lead, self.groove);
        match role {
            BlockRole::Full => {}
            BlockRole::Chords => {
                // All three pulses are about to hold a chord, so nothing
                // else may be using them.
                arr.lead = false;
                arr.harmony = false;
                arr.counter = None;
            }
            BlockRole::Break => {
                arr.lead = false;
                arr.counter = None;
                arr.saw = None;
                arr.shaker = false;
                arr.snare = false;
                arr.hat_k = 0;
                arr.kick_extra = &[];
                arr.four_floor = false;
                arr.harm_rate = HarmRate::HalfBar;
            }
        }
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
                    midi: clamp_midi(tonic + 24 + chord.pitch(mode, n.deg)),
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
            let base = clamp_midi(tonic + 12 + chord.pitch(mode, chord.degree));
            let (span, hits) = arr.harm_rate.grid(meter);
            if arr.harm_inst == Inst::Stab {
                // Offbeat stabs drive; a held pad would just sit there.
                // Half a span late, so they land between the kicks.
                for k in 0..hits {
                    out.push(NoteEvent {
                        at: at((k * span + span / 2) as f32),
                        inst: Inst::Stab,
                        midi: base,
                        vel: 96,
                        frames: frames(2),
                        arp: Some(arp),
                        glide: 0,
                    });
                }
            } else {
                for k in 0..hits {
                    out.push(NoteEvent {
                        at: at((k * span) as f32),
                        inst: arr.harm_inst,
                        midi: base,
                        vel: 84,
                        frames: frames(span as u8),
                        arp: Some(arp),
                        glide: 0,
                    });
                }
            }
        }

        // --- block chords ---------------------------------------------
        // Three pulses holding one voicing. Everywhere else in this file
        // a "chord" is one channel running an arpeggio macro fast enough
        // to imply one; here it is three notes actually sounding at
        // once, which is a different thing to listen to and the whole
        // reason these blocks exist.
        if role == BlockRole::Chords {
            let voicing = chord.voicing();
            let midi_of = |v: i32| clamp_midi(tonic + 12 + chord.pitch(mode, v));
            match self.chord_style {
                ChordStyle::Sustain => {
                    let (span, hits) = HarmRate::HalfBar.grid(meter);
                    for k in 0..hits {
                        for (i, inst) in [Inst::ChordLo, Inst::ChordMid, Inst::ChordHi]
                            .into_iter()
                            .enumerate()
                        {
                            out.push(NoteEvent {
                                at: at((k * span) as f32),
                                inst,
                                midi: midi_of(voicing[i]),
                                // Ring right up to the next strike: the
                                // point is the sustain, not the attack.
                                frames: frames(span as u8),
                                vel: 92,
                                arp: None,
                                glide: 0,
                            });
                        }
                    }
                }
                ChordStyle::Strum => {
                    let pattern = strum_pattern(meter, self.strum_busy);
                    let spread = (STRUM_SPREAD_SECS * SAMPLE_RATE as f32) as u64;
                    for (n, s) in pattern.iter().enumerate() {
                        // Ring until the next stroke, so the chord is
                        // continuous the way a strummed one is.
                        let next = pattern.get(n + 1).map_or(steps as f32, |x| x.step as f32)
                            - s.step as f32;
                        let len = (next.max(1.0) as u8).max(1);
                        // Down goes low string first and takes the whole
                        // chord; up comes back over the top two only.
                        let strings: &[usize] = if s.down { &[0, 1, 2] } else { &[2, 1] };
                        let base = at(s.step as f32);
                        for (order, i) in strings.iter().enumerate() {
                            out.push(NoteEvent {
                                at: base + order as u64 * spread,
                                inst: [Inst::StrumLo, Inst::StrumMid, Inst::StrumHi][*i],
                                midi: midi_of(voicing[*i]),
                                frames: frames(len),
                                // Upstrokes are lighter — that is the
                                // accent pattern doing the work.
                                vel: if s.down { 100 } else { 74 },
                                arp: None,
                                glide: 0,
                            });
                        }
                    }
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
                        midi: clamp_midi(tonic + 24 + chord.pitch(mode, n.deg)),
                        vel: 62,
                        frames: frames(n.len.min(3)),
                        arp: None,
                        glide: 0,
                    });
                }
            }
            Some(Counter::Arp) => {
                // The full ladder, not just the triad: on a seventh or a
                // ninth this is where the extra tone is most audible,
                // because it goes past once every bar instead of being
                // one third of a three-note macro.
                let (tones, n) = chord.ladder();
                for i in (0..steps).step_by(1) {
                    let deg = tones[i as usize % n] + 7;
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::Arp,
                        midi: clamp_midi(tonic + 12 + chord.pitch(mode, deg)),
                        vel: 72,
                        frames: frames(1),
                        arp: None,
                        glide: 0,
                    });
                }
            }
            Some(Counter::CutPiano) => {
                // Eleven sixteenths in sixteen, which Bjorklund spaces as
                // `x.xx.xx.xx.xx.xx` — a rest on the second sixteenth of
                // every beat. That one hole is the difference between a
                // figure and a machine gun, and it is why the count is
                // eleven rather than the twelve that would divide evenly.
                //
                // The chord is walked two octaves' worth rather than
                // circling one: the line has to climb or the ear hears a
                // loop instead of a hook.
                //
                // The notes are cut by the instrument, not by the length
                // asked for here: `CutPiano`'s envelope is over inside
                // five frames whatever it is handed.
                let (tones, n) = chord.ladder();
                let spbeat = meter.steps_per_beat() as usize;
                let mut taken = 0usize;
                // Rotated away from the hats, which are on the same
                // grid and would otherwise fuse into one sound wherever
                // the two patterns happen to agree.
                for (i, on) in euclid_rot(steps as usize * 11 / 16, steps as usize, hat_rot + 2)
                    .iter()
                    .enumerate()
                {
                    if !*on {
                        continue;
                    }
                    let octave = 7 * (1 + (taken / n) as i32 % 2);
                    out.push(NoteEvent {
                        at: at(i as f32),
                        inst: Inst::CutPiano,
                        midi: clamp_midi(tonic + 12 + chord.pitch(mode, tones[taken % n] + octave)),
                        vel: if i % spbeat == 0 { 96 } else { 74 },
                        frames: frames(1),
                        arp: None,
                        glide: 0,
                    });
                    taken += 1;
                }
            }
            _ => {}
        }

        // --- wavetable pad --------------------------------------------
        if let Some(pad) = arr.pad {
            out.push(NoteEvent {
                at: at(0.0),
                inst: pad,
                midi: clamp_midi(tonic + chord.pitch(mode, chord.degree)),
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
                midi: clamp_midi(tonic - 12 + chord.pitch(mode, deg)),
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
                        midi: clamp_midi(tonic + chord.pitch(mode, deg)),
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
                        midi: clamp_midi(tonic + 12 + chord.pitch(mode, n.deg)),
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
                    midi: clamp_midi(tonic + 12 + chord.pitch(mode, 7 - n + k)),
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
        // The one seventh in the table is the V that lands the piece back
        // home, and it is a real dominant: this is the last cadence of the
        // song, and a minor v7 does not close anything.
        let chord = if seventh {
            Chord::dom(degree, Ext::Seventh)
        } else {
            Chord::new(degree)
        };
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
                midi: clamp_midi(tonic + chord.pitch(mode, chord.degree)),
                vel: if last { 92 } else { 80 },
                frames: frames(steps as u8),
                arp: Some(arp),
                glide: 0,
            });
            out.push(NoteEvent {
                at: at(0.0),
                inst: Inst::Organ,
                midi: clamp_midi(tonic + 12 + chord.pitch(mode, chord.degree)),
                vel: 78,
                frames: frames(steps as u8),
                arp: Some(arp),
                glide: 0,
            });
        }
        out.push(NoteEvent {
            at: at(0.0),
            inst: Inst::Bass,
            midi: clamp_midi(tonic - 12 + chord.pitch(mode, chord.degree)),
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
        // Offbeats only, straight through: the beat itself belongs to the
        // kick, whose body is the note there. A bass that plays under the
        // kick as well as between it does not sound twice as heavy, it
        // sounds like one long note with a click on it.
        5 => (0..steps / spb)
            .map(|i| (i * spb + spb / 2, spb / 2, root))
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

    /// Material on the house groove, which is what every test that is
    /// not about the groove is asking for.
    fn material(seed: u64, profile: Profile, meter: Meter, smoothness: f32) -> Material {
        build_material(seed, profile, meter, smoothness, Groove::Normal)
    }

    fn material_with(
        seed: u64,
        profile: Profile,
        meter: Meter,
        smoothness: f32,
        groove: Groove,
    ) -> Material {
        build_material(seed, profile, meter, smoothness, groove)
    }

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
                let mat = material(12345, p, m, default_smoothness(p));
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
            let m = material(
                999,
                Profile::SoloCalm,
                meter,
                default_smoothness(Profile::SoloCalm),
            );
            for s in &m.sections {
                assert_eq!(s.chords[3].degree, 4, "antecedent must close on V");
                assert_eq!(s.chords[3].ext, Ext::Seventh);
                assert_eq!(s.chords[7].degree, 0, "consequent must close on I");
            }
        }
    }

    #[test]
    fn strong_beats_are_chord_tones() {
        let mut checked = 0;
        for meter in METERS {
            let m = material(
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
    fn snapping_does_not_hammer_one_note() {
        // Rhythm cell 0 puts all four onsets on the beat, so every note
        // in the bar is snapped to the chord. A walk that moves inside a
        // third then lands on the same tone every time, and the tune
        // comes out as a pulse rather than a line.
        let chord = Chord::new(0);
        let motif = Motif {
            rhythm: 0,
            contour: vec![4, 5, 4, 5],
        };
        let out = realize(&motif, Transform::None, chord, Meter::Four, None);
        assert_eq!(out.len(), RHYTHMS_4_4[0].len());
        for w in out.windows(2) {
            assert_ne!(w[0].deg, w[1].deg, "the line collapsed onto one note");
        }
        // Without giving up the rule that put it on the chord.
        for n in &out {
            assert!(chord.contains(n.deg), "degree {} left the chord", n.deg);
        }
        // A walk that really does repeat a note keeps its repeat.
        let flat = Motif {
            rhythm: 0,
            contour: vec![4, 4, 4, 4],
        };
        let out = realize(&flat, Transform::None, chord, Meter::Four, None);
        assert!(out.iter().all(|n| n.deg == 4), "a held note was moved");
    }

    #[test]
    fn material_is_reused_not_re_rolled() {
        // The form must revisit sections: eighty bars of music from
        // twenty-four bars of material keeps new material inside the
        // ~35% budget real songs sit in.
        let m = material(
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
                        let m = material(seed, profile, meter, smooth);
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
    fn a_melody_is_allowed_to_repeat_a_note() {
        // A walk that has to move on every onset never lets a note land,
        // which is the clearest tell of a generated line.
        let mut rng = Rng::new(0x5A1E);
        let mut same = 0.0;
        let mut total = 0.0;
        for _ in 0..2_000 {
            let c = contour(&mut rng, 8, 0.7);
            for w in c.windows(2) {
                total += 1.0;
                if w[0] == w[1] {
                    same += 1.0;
                }
            }
        }
        let share = same / total;
        assert!(
            (0.05..0.35).contains(&share),
            "repeated notes were {:.0}% of the line",
            share * 100.0
        );
    }

    #[test]
    fn a_leap_is_answered_by_a_step_back() {
        // Post-skip reversal. The melody may leap, but a leap that keeps
        // going the same way twice is how a line stops sounding like one.
        let mut rng = Rng::new(0x1EA0);
        let mut answered = 0.0;
        let mut leaps = 0.0;
        for smooth in [0.3, 0.6, 0.9] {
            for _ in 0..1_500 {
                let c = contour(&mut rng, 12, smooth);
                for i in 1..c.len() - 1 {
                    let leap = c[i] - c[i - 1];
                    if leap.abs() < 2 {
                        continue;
                    }
                    // The answer is the next move that goes anywhere.
                    let Some(next) = c[i + 1..].iter().map(|d| d - c[i]).find(|d| *d != 0) else {
                        continue;
                    };
                    leaps += 1.0;
                    if next.signum() != leap.signum() {
                        answered += 1.0;
                    }
                }
            }
        }
        assert!(leaps > 500.0, "the test barely saw a leap");
        let share = answered / leaps;
        assert!(
            share > 0.6,
            "only {:.0}% of leaps turned back",
            share * 100.0
        );
    }

    #[test]
    fn the_line_carries_over_the_bar_line() {
        // Inside a bar the walk is careful; the join used to throw that
        // away, because a motif starts wherever its own contour starts.
        let mut rng = Rng::new(0x5EA1);
        let mut linked = 0.0;
        let mut free = 0.0;
        let mut linked_wide = 0;
        let mut free_wide = 0;
        let mut n = 0.0;
        for i in 0..400 {
            let motif = Motif {
                rhythm: i % RHYTHMS_4_4.len(),
                contour: contour(&mut rng, RHYTHMS_4_4[i % RHYTHMS_4_4.len()].len(), 0.6),
            };
            let chord = Chord::new((i % 7) as i32);
            for prev in CONTOUR_LOW..=CONTOUR_HIGH {
                let a = realize(&motif, Transform::None, chord, Meter::Four, None);
                let b = realize(&motif, Transform::None, chord, Meter::Four, Some(prev));
                let (fs, ls) = ((a[0].deg - prev).abs(), (b[0].deg - prev).abs());
                free += fs as f32;
                linked += ls as f32;
                free_wide += (fs > SEAM_MAX) as i32;
                linked_wide += (ls > SEAM_MAX) as i32;
                n += 1.0;
            }
        }
        // The seams that were already inside a fifth are left alone, so
        // the mean only moves a little. The tail is the point: a join
        // wider than anything the contour itself would write has to be
        // most of the way gone.
        assert!(free_wide > 500, "the test never saw a wide seam");
        assert!(
            linked_wide * 4 < free_wide,
            "wide seams barely moved: {free_wide} -> {linked_wide}"
        );
        assert!(
            linked / n < (free / n) * 0.95,
            "the bar line is no smoother: {:.2} -> {:.2}",
            free / n,
            linked / n
        );
    }

    /// Mean melody onsets per bar for a profile, over many seeds.
    fn mean_onsets(profile: Profile) -> f32 {
        let mut total = 0.0;
        let mut bars = 0.0;
        for seed in 0..60u64 {
            let m = material(seed, profile, Meter::Four, default_smoothness(profile));
            for s in &m.sections {
                for notes in &s.melody {
                    total += notes.len() as f32;
                    bars += 1.0;
                }
            }
        }
        total / bars
    }

    /// Plan one whole form and report the roles it rolled plus, per
    /// block, how many notes each instrument played.
    fn blocks_of(profile: Profile, intensity: f32) -> (Vec<BlockRole>, Vec<Vec<(Inst, usize)>>) {
        blocks_of_seed(0x5EED, profile, intensity)
    }

    fn blocks_of_seed(
        seed: u64,
        profile: Profile,
        intensity: f32,
    ) -> (Vec<BlockRole>, Vec<Vec<(Inst, usize)>>) {
        let mut c = Composer::new(seed);
        c.set_profile(profile, intensity);
        let ctx = Context {
            profile,
            intensity,
            elapsed: 600.0,
            ..Default::default()
        };
        let roles = c.material.roles.clone();
        let mut out = Vec::new();
        let blocks = roles
            .iter()
            .map(|_| {
                let start = out.len();
                for _ in 0..BARS_PER_SECTION {
                    c.plan_bar(&ctx, &mut out);
                }
                let mut insts: Vec<(Inst, usize)> = Vec::new();
                for n in &out[start..] {
                    match insts.iter_mut().find(|(i, _)| *i == n.inst) {
                        Some((_, c)) => *c += 1,
                        None => insts.push((n.inst, 1)),
                    }
                }
                insts
            })
            .collect();
        (roles, blocks)
    }

    /// How many notes `pred` matched in a block.
    fn count(block: &[(Inst, usize)], pred: impl Fn(Inst) -> bool) -> usize {
        block
            .iter()
            .filter(|(i, _)| pred(*i))
            .map(|(_, c)| *c)
            .sum()
    }

    #[test]
    fn the_chord_blocks_rest_the_melody_and_hold_a_real_chord() {
        let (roles, blocks) = blocks_of(Profile::VsIntense, 0.7);
        let melody = |i: Inst| i.is_melody();
        let full_snares = count(&blocks[0], |i| i.is_snare());
        let full_hats = count(&blocks[0], |i| i == Inst::Hat || i == Inst::Shaker);

        for (i, role) in roles.iter().enumerate() {
            let b = &blocks[i];
            let has = |want: Inst| b.iter().any(|(x, _)| *x == want);
            match role {
                BlockRole::Chords => {
                    assert_eq!(
                        count(b, melody),
                        0,
                        "block {i} is a chord block, lead played"
                    );
                    // All three pulses, sounding together — that is what
                    // makes it a chord rather than an implied one.
                    // Whichever way this piece plays them, all three
                    // pulses have to be sounding.
                    let held = [Inst::ChordLo, Inst::ChordMid, Inst::ChordHi];
                    let strummed = [Inst::StrumLo, Inst::StrumMid, Inst::StrumHi];
                    assert!(
                        held.iter().all(|w| has(*w)) || strummed.iter().all(|w| has(*w)),
                        "block {i} is not holding a full chord"
                    );
                }
                BlockRole::Break => {
                    assert_eq!(count(b, melody), 0, "block {i} is a break, lead played");
                    // The backbeat stops. The fill in the last bar does
                    // not — that is the pickup back into the tune, and
                    // taking it away would make the return arrive from
                    // nowhere.
                    let snares = count(b, |x| x.is_snare());
                    assert!(
                        snares * 3 < full_snares,
                        "block {i} is a break but kept {snares} snare hits against {full_snares}"
                    );
                    let hats = count(b, |x| x == Inst::Hat || x == Inst::Shaker);
                    assert!(
                        hats * 5 < full_hats,
                        "block {i} is a break but kept {hats} hats against {full_hats}"
                    );
                }
                BlockRole::Full => {
                    assert!(count(b, melody) > 0, "block {i} is full but has no melody");
                    assert_eq!(
                        count(b, |x| x.is_chord_voice()),
                        0,
                        "block {i} should not hold block chords"
                    );
                }
            }
        }
    }

    #[test]
    fn the_melody_changes_instrument_partway_through() {
        // Not every seed rolls two different instruments for every
        // profile, but across a spread of them the swap has to show up.
        let mut swapped = 0;
        for seed in 0..25u64 {
            let mut c = Composer::new(seed);
            c.set_profile(Profile::VsIntense, 0.7);
            if c.lead_alt != c.lead_first() {
                swapped += 1;
            }
        }
        assert!(
            swapped >= 20,
            "only {swapped}/25 pieces rolled a second lead"
        );

        // And the second instrument really is what the later blocks use.
        let (_, blocks) = blocks_of(Profile::SoloCalm, 0.6);
        let melodies = |b: &Vec<(Inst, usize)>| -> Vec<Inst> {
            b.iter()
                .map(|(i, _)| *i)
                .filter(|i| i.is_melody())
                .collect()
        };
        let early = melodies(&blocks[0]);
        let late = melodies(&blocks[blocks.len() - 1]);
        assert!(!early.is_empty() && !late.is_empty());
        assert_ne!(early, late, "the melody never changed instrument");
    }

    #[test]
    fn a_strum_spreads_its_strings_in_time() {
        let mut c = Composer::new(3);
        c.set_profile(Profile::SoloCalm, 0.5);
        c.force_chord_style(Some(ChordStyle::Strum));
        let ctx = Context {
            profile: Profile::SoloCalm,
            intensity: 0.5,
            elapsed: 600.0,
            ..Default::default()
        };
        // Block 1 is the first chord block of the chordal form.
        let mut out = Vec::new();
        for _ in 0..(BARS_PER_SECTION * 2) {
            c.plan_bar(&ctx, &mut out);
        }
        let strums: Vec<&NoteEvent> = out.iter().filter(|e| e.inst.is_chord_voice()).collect();
        assert!(!strums.is_empty(), "no chord notes at all");
        assert!(
            strums
                .iter()
                .all(|e| matches!(e.inst, Inst::StrumLo | Inst::StrumMid | Inst::StrumHi)),
            "a pinned strum played held chords"
        );

        // Find one downstroke: low, then mid, then high, each a little
        // after the last. Simultaneous notes would mean no strum at all.
        let lo = strums
            .iter()
            .find(|e| e.inst == Inst::StrumLo)
            .expect("no low string");
        let mid = strums
            .iter()
            .find(|e| e.inst == Inst::StrumMid && e.at > lo.at)
            .expect("no middle string after the low one");
        let hi = strums
            .iter()
            .find(|e| e.inst == Inst::StrumHi && e.at > mid.at)
            .expect("no top string after the middle one");
        let spread = (STRUM_SPREAD_SECS * SAMPLE_RATE as f32) as u64;
        assert_eq!(mid.at - lo.at, spread);
        assert_eq!(hi.at - mid.at, spread);
        // And the whole gesture stays short enough to read as one chord
        // rather than as an arpeggio.
        assert!(
            ((hi.at - lo.at) as f32 / SAMPLE_RATE as f32) < 0.05,
            "the strum takes too long to cross the strings"
        );
    }

    #[test]
    fn upstrokes_are_lighter_and_skip_the_bass_string() {
        let mut c = Composer::new(11);
        c.set_profile(Profile::SoloCalm, 0.5);
        c.force_chord_style(Some(ChordStyle::Strum));
        let ctx = Context {
            profile: Profile::SoloCalm,
            intensity: 0.5,
            elapsed: 600.0,
            ..Default::default()
        };
        let mut out = Vec::new();
        for _ in 0..(BARS_PER_SECTION * 2) {
            c.plan_bar(&ctx, &mut out);
        }
        let lows = out.iter().filter(|e| e.inst == Inst::StrumLo).count();
        let highs = out.iter().filter(|e| e.inst == Inst::StrumHi).count();
        assert!(lows > 0 && highs > 0);
        assert!(
            highs > lows,
            "upstrokes should skip the bass string: {lows} low vs {highs} high"
        );
        // Two velocities, and the quiet one is the upstroke.
        let mut vels: Vec<u8> = out
            .iter()
            .filter(|e| e.inst == Inst::StrumHi)
            .map(|e| e.vel)
            .collect();
        vels.sort_unstable();
        vels.dedup();
        assert_eq!(vels.len(), 2, "strums should be accented, got {vels:?}");
    }

    #[test]
    fn every_form_ends_on_a_full_block() {
        // Swept over seeds now that the form is rolled rather than picked
        // from one of two tables: these are the properties every roll has
        // to come out with, whichever shape it lands on.
        for profile in Profile::ALL {
            for seed in 0..40u64 {
                let m = material(seed, profile, Meter::Four, default_smoothness(profile));
                let roles = &m.roles;
                let what = format!("{profile:?} seed {seed}");
                assert_eq!(roles.len(), m.form.len(), "{what}: a block with no role");
                // Block 0 states the tune, and the last block is the
                // final chorus, which forces every layer on. A rest or a
                // chord block at either end would be wasted.
                assert_eq!(roles[0], BlockRole::Full, "{what}");
                assert_eq!(roles[roles.len() - 1], BlockRole::Full, "{what}");
                // And there has to be some contrast in there at all.
                assert!(roles.contains(&BlockRole::Chords), "{what}");
                assert!(roles.contains(&BlockRole::Break), "{what}");
                // The breakdown is the air before the last chorus, not a
                // hole near the top of the piece — and not the block that
                // owes the piece a run-up fill either.
                for (i, r) in roles.iter().enumerate() {
                    if *r == BlockRole::Break {
                        assert!(i * 2 >= roles.len(), "{what}: early breakdown at {i}");
                        assert!(
                            i + 2 < roles.len(),
                            "{what}: breakdown on the run-up at {i}"
                        );
                    }
                }
                // Something has to still play the tune.
                let full = roles.iter().filter(|r| **r == BlockRole::Full).count();
                assert!(full >= 5, "{what} only has {full} blocks with a melody");
            }
        }
    }

    #[test]
    fn the_form_itself_varies_between_pieces() {
        // The thing this whole exercise was for: two pieces should not
        // break down in the same bar just because they share a profile.
        let mut shapes = HashSet::new();
        let mut lengths = HashSet::new();
        for seed in 0..40u64 {
            let m = material(
                seed,
                Profile::VsIntense,
                Meter::Four,
                default_smoothness(Profile::VsIntense),
            );
            lengths.insert(m.form.len());
            shapes.insert(format!("{:?}{:?}", m.form, m.roles));
        }
        assert!(
            shapes.len() >= 20,
            "only {} distinct forms in 40 pieces",
            shapes.len()
        );
        assert!(lengths.len() > 1, "every piece is the same length");
    }

    #[test]
    fn the_solo_profiles_give_the_chords_more_room_than_versus() {
        let chordy = |p: Profile| {
            let mut total = 0.0;
            for seed in 0..40u64 {
                let m = material(seed, p, Meter::Four, default_smoothness(p));
                total += m.roles.iter().filter(|r| **r == BlockRole::Chords).count() as f32
                    / m.roles.len() as f32;
            }
            total / 40.0
        };
        assert!(
            chordy(Profile::SoloCalm) > chordy(Profile::VsIntense),
            "solo should be the chord-forward one"
        );
        // Both calm profiles draw from the same budget, so they land in
        // the same place over a spread of seeds.
        assert!((chordy(Profile::Zen) - chordy(Profile::SoloCalm)).abs() < 0.05);
    }

    #[test]
    fn no_profile_writes_a_denser_tune_than_another() {
        // Melody density is deliberately uniform: the note count is not
        // where a profile's character lives, and a relentless tune in
        // versus left no room to build. Contrast comes from the form.
        let vs = mean_onsets(Profile::VsIntense);
        let zen = mean_onsets(Profile::Zen);
        assert!(
            (vs - zen).abs() < 0.35,
            "versus {vs:.2} and zen {zen:.2} should write equally dense tunes"
        );
    }

    #[test]
    fn versus_progressions_are_mostly_extended() {
        // The point of the exercise: versus should be stacking thirds,
        // and the calm profiles should mostly not be.
        let count = |profile: Profile| {
            let mut extended = 0;
            let mut total = 0;
            for seed in 0..40u64 {
                let m = material(seed, profile, Meter::Four, default_smoothness(profile));
                for s in &m.sections {
                    for c in &s.chords {
                        total += 1;
                        if c.ext != Ext::Triad {
                            extended += 1;
                        }
                    }
                }
            }
            extended as f32 / total as f32
        };
        let vs = count(Profile::VsIntense);
        let zen = count(Profile::Zen);
        assert!(
            vs > 0.6,
            "versus only extended {:.0}% of chords",
            vs * 100.0
        );
        assert!(zen < 0.4, "zen extended {:.0}% of chords", zen * 100.0);
    }

    #[test]
    fn the_cadence_bars_keep_their_own_chords() {
        // Bar 3 is the dominant seventh that pulls home and bar 7 is the
        // arrival; colour must not be sprinkled onto either.
        for profile in Profile::ALL {
            for seed in 0..30u64 {
                let m = material(seed, profile, Meter::Four, default_smoothness(profile));
                for s in &m.sections {
                    assert_eq!(s.chords[3].degree, 4);
                    assert_eq!(s.chords[3].ext, Ext::Seventh);
                    assert_eq!(s.chords[7].degree, 0);
                    assert_eq!(s.chords[7].ext, Ext::Triad);
                }
            }
        }
    }

    #[test]
    fn the_harmony_grid_tiles_every_meter_exactly() {
        for meter in METERS {
            for rate in [HarmRate::HalfBar, HarmRate::Beat, HarmRate::Eighth] {
                let (span, hits) = rate.grid(meter);
                assert!(span >= 1, "{rate:?} in {} has no span", meter.name());
                assert!(hits >= 1);
                // The last hit must start inside the bar.
                assert!(
                    (hits - 1) * span < meter.steps(),
                    "{rate:?} in {} runs past the bar line",
                    meter.name()
                );
            }
            // And it really is a ladder of increasing density.
            let (_, half) = HarmRate::HalfBar.grid(meter);
            let (_, beat) = HarmRate::Beat.grid(meter);
            let (_, eighth) = HarmRate::Eighth.grid(meter);
            assert!(half <= beat && beat <= eighth, "{}", meter.name());
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
                        Mode::MelodicMinor,
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
    fn calm_is_slower_than_versus_and_both_stay_minor() {
        let calm = tempo_range(Profile::SoloCalm, Groove::Normal);
        let versus = tempo_range(Profile::VsIntense, Groove::Normal);
        assert!(calm.1 < versus.0);
        // Everything that plays during a match is minor, whichever lane
        // of the palette the piece rolled; only the fanfare is allowed
        // to be cheerful.
        for p in [Profile::Ambient, Profile::SoloCalm, Profile::VsIntense] {
            for band in 0..4 {
                for m in mode_palette(p, band) {
                    assert!(
                        m.is_minor(),
                        "{p:?} band {band} offers {m:?}, which is major"
                    );
                }
            }
        }
        for m in mode_palette(Profile::Victory, 0) {
            assert!(!m.is_minor(), "the fanfare offers {m:?}");
        }
        // Solo and versus deliberately share scales now — the difference
        // between them is tempo and arrangement, not which minor. What
        // must hold is that neither wanders out of minor, above.
    }

    #[test]
    fn versus_never_reaches_for_phrygian() {
        // A flat second is a very particular flavour, and it is not the
        // one a match at its hottest wants. Harmonic and melodic minor
        // raise the seventh instead, which pulls rather than colours.
        for band in 0..4 {
            for m in mode_palette(Profile::VsIntense, band) {
                assert!(
                    !matches!(m, Mode::Phrygian | Mode::PhrygianDominant),
                    "versus band {band} offers {m:?}"
                );
            }
        }
    }

    #[test]
    fn every_band_offers_a_choice_of_scale() {
        // The whole point of the palettes: two pieces at the same danger
        // level should be able to be in different scales.
        for p in Profile::ALL {
            for band in 0..4 {
                let palette = mode_palette(p, band);
                assert!(!palette.is_empty(), "{p:?} band {band} has no scale");
                // Menus are allowed to sit still; the playing profiles
                // are not.
                if p != Profile::Ambient && p != Profile::Victory {
                    assert!(
                        palette.len() >= 2,
                        "{p:?} band {band} has only one scale, so every piece sounds alike"
                    );
                }
                // A lane index past the end must still land somewhere.
                for v in 0..8 {
                    assert!(palette.contains(&mode_target(p, band as f32 * 0.34, v)));
                }
            }
        }
    }

    #[test]
    fn a_piece_holds_its_lane_but_pieces_differ() {
        // Within one piece the scale must be a function of intensity
        // alone, or it would flicker between phrases at a steady danger
        // level. Across pieces it must not be.
        let mut seen: Vec<Mode> = Vec::new();
        for seed in 0..30u64 {
            let mut c = Composer::new(seed);
            c.set_profile(Profile::VsIntense, 0.9);
            let m = c.mode;
            // Same piece, same intensity, same answer — whether the mode
            // came off the ladder or was pinned by the progression.
            assert_eq!(c.piece_mode(0.9), m);
            if !seen.contains(&m) {
                seen.push(m);
            }
        }
        assert!(
            seen.len() >= 2,
            "30 versus pieces at full danger all chose {seen:?}"
        );
    }

    /// A piece on the hard-bass groove, planned for `bars`.
    fn hard_bass(
        seed: u64,
        profile: Profile,
        intensity: f32,
        bars: u32,
    ) -> (Composer, Vec<NoteEvent>) {
        let mut c = Composer::new(seed);
        c.set_profile(profile, intensity);
        c.force_groove(Some(Groove::HardBass));
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
        (c, out)
    }

    #[test]
    fn the_maru_sa_changes_come_out_as_the_chords_they_name() {
        // In A minor: Am7 C7 FM7 E7, twice, and home on the last bar.
        // Written as pitch classes above the tonic, which is the only
        // form that can catch a borrowed third going missing.
        let m = material_with(
            0xD0BA,
            Profile::VsIntense,
            Meter::Four,
            0.5,
            Groove::HardBass,
        );
        assert!(m.marusa, "the hard-bass groove did not take the changes");
        let spell = |c: &Chord| {
            let root = MARUSA_MODE.pitch(c.degree);
            let mut pcs: Vec<i32> = std::iter::once(root)
                .chain(c.arp(MARUSA_MODE).iter().map(|o| root + *o as i32))
                .map(|p| p.rem_euclid(12))
                .collect();
            pcs.sort_unstable();
            pcs.dedup();
            pcs
        };
        for s in &m.sections {
            for half in 0..2 {
                assert_eq!(spell(&s.chords[half * 4]), vec![0, 3, 7, 10], "i7");
                assert_eq!(spell(&s.chords[half * 4 + 1]), vec![1, 3, 7, 10], "bIII7");
                assert_eq!(spell(&s.chords[half * 4 + 2]), vec![0, 3, 7, 8], "bVIM7");
            }
            // The half cadence is a real dominant: a major third (11 = the
            // leading tone) where the key has a minor one.
            assert_eq!(spell(&s.chords[3]), vec![2, 5, 7, 11], "V7");
            // And the last bar is home, plain.
            assert_eq!(spell(&s.chords[7]), vec![0, 3, 7], "i");
        }
    }

    #[test]
    fn a_piece_on_those_changes_stays_in_the_one_mode_that_spells_them() {
        // The maru-sa loop needs a minor sixth; Dorian and the bright
        // modes do not have one, so the intensity ladder has to stand
        // down for these pieces. It must stand down *completely* — a mode
        // that changed at the next phrase boundary would take the
        // progression apart mid-piece.
        let (mut c, _) = hard_bass(0xD0BA, Profile::VsIntense, 0.1, 8);
        assert!(c.material.marusa);
        for intensity in [0.0f32, 0.35, 0.7, 1.0] {
            assert_eq!(c.piece_mode(intensity), MARUSA_MODE);
        }
        // A piece that did not take them keeps its ladder.
        c.force_groove(Some(Groove::Normal));
        if !c.material.marusa {
            assert_eq!(c.piece_mode(0.9), mode_target(c.profile, 0.9, c.mode_variant));
        }
    }

    #[test]
    fn the_hard_bass_groove_leaves_the_beat_to_the_kick() {
        for meter in METERS {
            let (mut c, _) = hard_bass(0x8A55, Profile::VsIntense, 0.8, 0);
            c.force_meter(meter);
            let ctx = Context {
                profile: Profile::VsIntense,
                intensity: 0.8,
                elapsed: 600.0,
                ..Default::default()
            };
            let mut out = Vec::new();
            // One block, so no breakdown or chord block gets in the way.
            for _ in 0..BARS_PER_SECTION {
                c.plan_bar(&ctx, &mut out);
            }
            let spb = samples_per_step(c.bpm);
            let step_of = |e: &NoteEvent| {
                let bar = (spb * meter.steps() as f64) as u64;
                ((e.at % bar) as f64 / spb).round() as u32 % meter.steps()
            };
            let beat = meter.steps_per_beat();

            let kicks: Vec<u32> = out
                .iter()
                .filter(|e| e.inst.is_kick())
                .map(step_of)
                .collect();
            assert!(!kicks.is_empty());
            for b in 0..meter.beats() {
                assert!(kicks.contains(&(b * beat)), "no kick on beat {b}");
            }
            // The bass answers between the kicks and never under them.
            let bass: Vec<u32> = out
                .iter()
                .filter(|e| e.inst == Inst::Bass)
                .map(step_of)
                .collect();
            assert!(bass.len() >= meter.beats() as usize);
            for step in &bass {
                assert!(
                    !step.is_multiple_of(beat),
                    "the bass landed on the beat at step {step} in {}",
                    meter.name()
                );
            }
            // Nothing swings; a shuffled sixteenth under this is a
            // different genre.
            assert_eq!(
                arrange(Profile::VsIntense, &ctx, Inst::Pluck, Groove::HardBass).swing,
                0.5
            );
        }
    }

    #[test]
    fn release_cut_piano_fills_the_sixteenths_without_ringing() {
        let (c, out) = hard_bass(0x9111, Profile::VsIntense, 0.7, BARS_PER_SECTION);
        let piano: Vec<&NoteEvent> = out.iter().filter(|e| e.inst == Inst::CutPiano).collect();
        assert!(
            piano.len() >= 6 * BARS_PER_SECTION as usize,
            "only {} cut-piano notes in eight bars",
            piano.len()
        );
        // Nothing is held: the longest of them is one sixteenth, and the
        // instrument's own envelope is over inside five frames whatever
        // it is handed.
        let step = frames_per_step(c.bpm).ceil() as u16;
        assert!(piano.iter().all(|e| e.frames <= step.max(2)));
        assert!(crate::inst_def(Inst::CutPiano).vol.loop_at.is_none());
        assert!(crate::inst_def(Inst::CutPiano).vol.v.len() <= 6);
        // And it is a figure, not a wall: some sixteenths are empty.
        let steps = c.meter.steps();
        assert!(piano.len() < steps as usize * BARS_PER_SECTION as usize);
    }

    #[test]
    fn only_the_profiles_that_want_a_beat_ever_roll_the_hard_groove() {
        let mut seen: Vec<(Profile, bool)> = Vec::new();
        for profile in Profile::ALL {
            let mut hard = false;
            let mut normal = false;
            for seed in 0..40u64 {
                let mut c = Composer::new(seed);
                c.set_profile(profile, 0.5);
                match c.groove() {
                    Groove::HardBass => hard = true,
                    Groove::Normal => normal = true,
                }
                // The groove and the kit agree, always.
                assert_eq!(
                    c.kit == Kit::Hard,
                    c.groove() == Groove::HardBass,
                    "{profile:?} rolled {:?} with {:?}",
                    c.groove(),
                    c.kit
                );
            }
            assert!(normal, "{profile:?} never writes an ordinary piece");
            seen.push((profile, hard));
        }
        for (profile, hard) in seen {
            assert_eq!(
                hard,
                hard_bass_chance(profile) > 0.0,
                "{profile:?} disagrees with its own odds"
            );
        }
    }

    #[test]
    fn pinning_the_groove_rebuilds_the_piece_around_it() {
        let mut c = Composer::new(0x6EED);
        c.set_profile(Profile::VsIntense, 0.6);
        let loose = (c.bpm, c.kit, c.groove());

        c.force_groove(Some(Groove::HardBass));
        assert_eq!(c.groove(), Groove::HardBass);
        assert!(c.pumps());
        assert_eq!(c.kit, Kit::Hard);
        // The odd meters are counted in sixteenths and get pulled back,
        // the same way every other tempo here does.
        let (lo, hi) = HARD_BASS_BPM;
        let scale = c.meter.tempo_scale();
        assert!(
            (lo * scale..=hi * scale).contains(&c.bpm),
            "{} BPM is not hard bass in {}",
            c.bpm,
            c.meter.name()
        );

        // A pin survives the next piece too.
        c.set_profile(Profile::SoloCalm, 0.3);
        assert_eq!(c.groove(), Groove::HardBass);
        assert_eq!(c.kit, Kit::Hard);

        // And letting it go hands the piece back exactly what its own roll
        // would have given it — same piece, so the same draws.
        let mut c = Composer::new(0x6EED);
        c.set_profile(Profile::VsIntense, 0.6);
        c.force_groove(Some(Groove::HardBass));
        c.force_groove(None);
        assert_eq!((c.bpm, c.kit, c.groove()), loose);
        assert_eq!(c.pumps(), loose.2 == Groove::HardBass);
    }

    #[test]
    fn versus_kicks_on_every_beat() {
        let ctx = Context {
            profile: Profile::VsIntense,
            intensity: 0.9,
            elapsed: 600.0,
            ..Default::default()
        };
        assert!(arrange(Profile::VsIntense, &ctx, Inst::Pluck, Groove::Normal).four_floor);

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
            arrange(Profile::SoloCalm, &ctx, Inst::Pluck, Groove::Normal)
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
        let early = arrange(Profile::VsIntense, &ctx(0.0, 0.1, false), Inst::Pluck, Groove::Normal);
        assert!(early.counter.is_none() && early.saw.is_none() && !early.perc);
        // Everything by the time it has been running a while and the
        // stack is high.
        let late = arrange(
            Profile::VsIntense,
            &ctx(120.0, 0.9, false),
            Inst::Pluck,
            Groove::Normal,
        );
        assert!(late.counter.is_some() && late.saw.is_some() && late.pad.is_some());
        assert!(late.shaker && late.perc && late.lead);
        // The zone strips it back to a held chord over the bass.
        let zone = arrange(Profile::VsIntense, &ctx(120.0, 0.9, true), Inst::Pluck, Groove::Normal);
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
        // "Every voice" includes the console's own noise channel, and the
        // kits that put their drums on the sample channel leave it empty
        // by design — see `a_full_arrangement_reaches_every_voice`.
        c.force_kit(Kit::Chip);
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
            Mode::MelodicMinor,
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
        let a = arrange(Profile::VsIntense, &ctx, Inst::Pluck, Groove::Normal);
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
            let a = material(
                0xABCD,
                Profile::SoloCalm,
                meter,
                default_smoothness(Profile::SoloCalm),
            );
            let b = material(
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

