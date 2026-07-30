//! The music theory the composer works in: modes, scale degrees, chords
//! and Euclidean rhythms.
//!
//! Everything the composer generates is stored as *scale degrees*, never
//! as absolute pitch. That is what lets the mode change with the danger
//! level without regenerating a single note — the same material is simply
//! read through a different interval table.

/// Diatonic (and two not-quite-diatonic) modes, ordered roughly bright to
/// dark. The intensity ladder walks down this list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Lydian,
    Ionian,
    Mixolydian,
    Dorian,
    Aeolian,
    Phrygian,
    HarmonicMinor,
    /// Ascending melodic minor: a minor third under a major sixth *and*
    /// a leading tone. Tense without being exotic, which is exactly the
    /// gap between Aeolian and harmonic minor.
    MelodicMinor,
    PhrygianDominant,
}

impl Mode {
    pub fn intervals(self) -> [i32; 7] {
        match self {
            Mode::Lydian => [0, 2, 4, 6, 7, 9, 11],
            Mode::Ionian => [0, 2, 4, 5, 7, 9, 11],
            Mode::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Mode::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Mode::Aeolian => [0, 2, 3, 5, 7, 8, 10],
            Mode::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Mode::HarmonicMinor => [0, 2, 3, 5, 7, 8, 11],
            Mode::MelodicMinor => [0, 2, 3, 5, 7, 9, 11],
            Mode::PhrygianDominant => [0, 1, 4, 5, 7, 8, 10],
        }
    }

    /// Semitones above the tonic for a scale degree, wrapping octaves for
    /// degrees outside 0..7 (including negative ones).
    pub fn pitch(self, degree: i32) -> i32 {
        let oct = degree.div_euclid(7);
        let idx = degree.rem_euclid(7) as usize;
        self.intervals()[idx] + 12 * oct
    }

    pub fn is_minor(self) -> bool {
        self.pitch(2) - self.pitch(0) == 3
    }

    /// The nearest mode with a flat seventh, keeping the third as it is.
    ///
    /// The outro's pivot depends on the seventh degree sitting ten
    /// semitones above the tonic; harmonic minor raises it to a leading
    /// tone, which is exactly the wrong interval. Softening the mode for
    /// four bars of wind-down costs nothing.
    pub fn with_flat_seventh(self) -> Mode {
        if self.pitch(6) == 10 {
            return self;
        }
        if self.is_minor() {
            Mode::Aeolian
        } else {
            Mode::Mixolydian
        }
    }

    /// English name for the "now playing" toast.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Lydian => "Lydian",
            Mode::Ionian => "major",
            Mode::Mixolydian => "Mixolydian",
            Mode::Dorian => "Dorian",
            Mode::Aeolian => "minor",
            Mode::Phrygian => "Phrygian",
            Mode::HarmonicMinor => "harmonic minor",
            Mode::MelodicMinor => "melodic minor",
            Mode::PhrygianDominant => "Phrygian dominant",
        }
    }
}

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A chord as a scale degree of the current mode (0 = I, 4 = V, ...).
/// How far up the stack of thirds a chord is built.
///
/// Degrees, not semitones — the mode decides whether the seventh is
/// major or minor, so a ii7 in Dorian comes out minor and a V7 comes out
/// dominant without any of this having to know which.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Ext {
    #[default]
    Triad,
    Seventh,
    Ninth,
}

/// How a chord colours its own thirds, where the mode is not the whole
/// story.
///
/// Everything else here is diatonic on purpose: a chord is a stack of
/// thirds read through the mode, which is what lets the danger level
/// darken the scale without touching a single stored note. The secondary
/// dominant is the one idiom that cannot be said that way. Its major third
/// and minor seventh are borrowed from outside the key, and they are the
/// entire reason it pulls — a `V7` in Aeolian is a minor seventh chord and
/// pulls nowhere at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Quality {
    /// Whatever the mode says.
    #[default]
    Mode,
    /// A major third, and a flat seventh if the chord has a seventh.
    Dominant,
}

/// We never spell chords absolutely: `degree` plus the mode is enough —
/// plus, for the two chords that borrow from outside the key, a
/// [`Quality`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub degree: i32,
    pub ext: Ext,
    pub quality: Quality,
}

impl Chord {
    pub const fn new(degree: i32) -> Self {
        Chord {
            degree,
            ext: Ext::Triad,
            quality: Quality::Mode,
        }
    }

    pub const fn with(degree: i32, ext: Ext) -> Self {
        Chord {
            degree,
            ext,
            quality: Quality::Mode,
        }
    }

    /// A secondary dominant on `degree`: major third, flat seventh,
    /// whatever the mode would otherwise have said.
    pub const fn dom(degree: i32, ext: Ext) -> Self {
        Chord {
            degree,
            ext,
            quality: Quality::Dominant,
        }
    }

    /// Semitones above the tonic for `degree`, with this chord's own
    /// colouring applied.
    ///
    /// Anything sounding *against* this chord should read pitch through
    /// here rather than through [`Mode::pitch`] — not only the chord
    /// voices but the melody too. A tune that lands on the diatonic third
    /// while the chord underneath it plays a raised one is not a
    /// suspension, it is a wrong note.
    pub fn pitch(&self, mode: Mode, degree: i32) -> i32 {
        mode.pitch(degree) + self.colour(mode, degree)
    }

    /// Semitones `degree` moves by, given what this chord is doing to its
    /// own thirds. Octave-invariant: the alteration is a property of the
    /// degree's place in the chord, so it applies wherever it is sung.
    fn colour(&self, mode: Mode, degree: i32) -> i32 {
        if self.quality != Quality::Dominant {
            return 0;
        }
        let root = mode.pitch(self.degree);
        match (degree - self.degree).rem_euclid(7) {
            // The third is what makes it dominant, seventh or no seventh.
            2 => root + 4 - mode.pitch(self.degree + 2),
            // The seventh is only there to be flattened if the chord has
            // one; on a plain triad this degree is a passing note.
            6 if self.ext != Ext::Triad => root + 10 - mode.pitch(self.degree + 6),
            _ => 0,
        }
    }

    /// Scale degrees of the triad, relative to the tonic.
    pub fn tones(&self) -> [i32; 3] {
        [self.degree, self.degree + 2, self.degree + 4]
    }

    /// The degrees an arpeggio should walk, and how many of them are
    /// real. Extended chords drop the root: the bass is already playing
    /// it, and four notes crammed into three slots would cost the third
    /// or the seventh — the two that actually say what the chord is.
    pub fn ladder(&self) -> ([i32; 4], usize) {
        let d = self.degree;
        match self.ext {
            Ext::Triad => ([d, d + 2, d + 4, 0], 3),
            Ext::Seventh => ([d, d + 2, d + 4, d + 6], 4),
            // 3rd, 5th, 7th, 9th — a rootless voicing.
            Ext::Ninth => ([d + 2, d + 4, d + 6, d + 8], 4),
        }
    }

    /// True if `degree` (any octave) is a chord tone. The melody snaps
    /// to these on strong beats, so widening the chord widens what the
    /// tune is allowed to land on — which is most of where the extra
    /// colour comes from.
    pub fn contains(&self, degree: i32) -> bool {
        let d = (degree - self.degree).rem_euclid(7);
        match self.ext {
            Ext::Triad => d == 0 || d == 2 || d == 4,
            Ext::Seventh => d == 0 || d == 2 || d == 4 || d == 6,
            // The ninth is the second degree an octave up; in pitch-class
            // terms that is simply degree 1.
            Ext::Ninth => d == 0 || d == 2 || d == 4 || d == 6 || d == 1,
        }
    }

    /// Nearest chord tone to `degree`, preferring to stay put on a tie.
    pub fn snap(&self, degree: i32) -> i32 {
        if self.contains(degree) {
            return degree;
        }
        let mut best = degree;
        let mut best_d = i32::MAX;
        for delta in -3..=3 {
            let cand = degree + delta;
            if self.contains(cand) && delta.abs() < best_d {
                best_d = delta.abs();
                best = cand;
            }
        }
        best
    }

    /// The three degrees a block chord should actually sound, low to
    /// high — the same selection [`Chord::arp`] implies, but as absolute
    /// degrees so they can be handed to three separate channels.
    pub fn voicing(&self) -> [i32; 3] {
        let (ladder, n) = self.ladder();
        if n == 4 {
            [ladder[1], ladder[2], ladder[3]]
        } else {
            [ladder[0], ladder[1], ladder[2]]
        }
    }

    /// Semitone offsets above the chord's own root, for the three-slot
    /// arpeggio macro the hardware voices run. Read through the mode, so
    /// a i chord really is minor and a V really is major.
    ///
    /// A four-note chord has one more tone than the macro has slots. The
    /// root is what gets dropped — the bass has it covered, and the
    /// third and the seventh are what make a seventh chord a seventh
    /// chord.
    pub fn arp(&self, mode: Mode) -> [i8; 3] {
        let root = mode.pitch(self.degree);
        let (ladder, n) = self.ladder();
        let take = if n == 4 { &ladder[1..4] } else { &ladder[0..3] };
        let mut out = [0i8; 3];
        for (o, deg) in out.iter_mut().zip(take) {
            *o = (self.pitch(mode, *deg) - root) as i8;
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Euclidean rhythm
// ---------------------------------------------------------------------------

/// Bjorklund's algorithm: distribute `k` onsets as evenly as possible over
/// `n` steps.
///
/// The widely repeated one-liner `(i * k) % n < k` is *not* this — it
/// disagrees with Bjorklund for many musically important inputs (E(5,8)
/// comes out `x.x.xx.x` instead of the cinquillo `x.xx.xx.`), so we run
/// the real thing.
pub fn euclid(k: usize, n: usize) -> Vec<bool> {
    if n == 0 {
        return Vec::new();
    }
    if k == 0 {
        return vec![false; n];
    }
    if k >= n {
        return vec![true; n];
    }
    // Groups start as k "1"s and (n-k) "0"s; repeatedly distribute the
    // remainder groups onto the front groups until at most one is left.
    let mut a: Vec<Vec<bool>> = (0..k).map(|_| vec![true]).collect();
    let mut b: Vec<Vec<bool>> = (0..n - k).map(|_| vec![false]).collect();
    while b.len() > 1 {
        let pairs = a.len().min(b.len());
        let mut next_a = Vec::with_capacity(pairs);
        for i in 0..pairs {
            let mut g = a[i].clone();
            g.extend_from_slice(&b[i]);
            next_a.push(g);
        }
        let next_b: Vec<Vec<bool>> = if a.len() > pairs {
            a[pairs..].to_vec()
        } else {
            b[pairs..].to_vec()
        };
        a = next_a;
        b = next_b;
        if a.len() <= 1 {
            break;
        }
    }
    let mut out = Vec::with_capacity(n);
    for g in a.iter().chain(b.iter()) {
        out.extend_from_slice(g);
    }
    out
}

/// [`euclid`] rotated left by `by` steps — rotation is what turns the raw
/// pattern into a groove that lands on beat one.
pub fn euclid_rot(k: usize, n: usize, by: usize) -> Vec<bool> {
    let p = euclid(k, n);
    if p.is_empty() {
        return p;
    }
    let by = by % p.len();
    let mut out = Vec::with_capacity(p.len());
    out.extend_from_slice(&p[by..]);
    out.extend_from_slice(&p[..by]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(p: &[bool]) -> String {
        p.iter().map(|&b| if b { 'x' } else { '.' }).collect()
    }

    #[test]
    fn euclid_matches_the_canonical_patterns() {
        // Toussaint's table. E(5,8) is the Cuban cinquillo; the popular
        // closed-form approximation gets this one wrong.
        assert_eq!(s(&euclid(5, 8)), "x.xx.xx.");
        assert_eq!(s(&euclid(3, 8)), "x..x..x.");
        assert_eq!(s(&euclid(2, 5)), "x.x..");
        assert_eq!(s(&euclid(4, 16)), "x...x...x...x...");
        assert_eq!(s(&euclid(7, 16)), "x..x.x.x..x.x.x.");
    }

    #[test]
    fn euclid_edges() {
        assert_eq!(s(&euclid(0, 4)), "....");
        assert_eq!(s(&euclid(4, 4)), "xxxx");
        assert_eq!(s(&euclid(9, 4)), "xxxx");
        assert!(euclid(3, 0).is_empty());
    }

    #[test]
    fn euclid_keeps_the_onset_count() {
        for n in 1..=32 {
            for k in 0..=n {
                let p = euclid(k, n);
                assert_eq!(p.len(), n);
                assert_eq!(p.iter().filter(|&&b| b).count(), k);
            }
        }
    }

    #[test]
    fn rotation_preserves_onsets() {
        let p = euclid_rot(5, 16, 3);
        assert_eq!(p.iter().filter(|&&b| b).count(), 5);
        assert_eq!(euclid_rot(5, 16, 0), euclid(5, 16));
    }

    #[test]
    fn mode_degrees_wrap_octaves() {
        assert_eq!(Mode::Ionian.pitch(0), 0);
        assert_eq!(Mode::Ionian.pitch(7), 12);
        assert_eq!(Mode::Ionian.pitch(-1), -1); // leading tone below
        assert_eq!(Mode::Aeolian.pitch(2), 3);
        assert!(Mode::Aeolian.is_minor());
        assert!(!Mode::Ionian.is_minor());
    }

    #[test]
    fn flattening_the_seventh_keeps_the_third() {
        for m in [
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
            let f = m.with_flat_seventh();
            assert_eq!(f.pitch(6), 10, "{m:?} -> {f:?} still has a leading tone");
            assert_eq!(f.is_minor(), m.is_minor(), "{m:?} changed quality");
        }
        // Modes that already have one are left alone.
        assert_eq!(Mode::Dorian.with_flat_seventh(), Mode::Dorian);
        assert_eq!(Mode::HarmonicMinor.with_flat_seventh(), Mode::Aeolian);
        assert_eq!(Mode::Ionian.with_flat_seventh(), Mode::Mixolydian);
    }

    #[test]
    fn chord_snapping_finds_the_nearest_tone() {
        let c = Chord::new(0);
        assert_eq!(c.snap(0), 0);
        assert_eq!(c.snap(2), 2);
        assert_eq!(c.snap(1), 0); // tie -> lower
        assert_eq!(c.snap(3), 2);
        assert_eq!(c.snap(8), 7); // an octave up still snaps
    }

    #[test]
    fn chord_arps_are_minor_or_major_by_mode() {
        // i in Aeolian is minor: root, +3, +7.
        assert_eq!(Chord::new(0).arp(Mode::Aeolian), [0, 3, 7]);
        // V in harmonic minor is major: root, +4, +7.
        assert_eq!(Chord::new(4).arp(Mode::HarmonicMinor), [0, 4, 7]);
        // I in Ionian is major.
        assert_eq!(Chord::new(0).arp(Mode::Ionian), [0, 4, 7]);
    }
}

#[cfg(test)]
mod ext_tests {
    use super::*;

    #[test]
    fn extensions_widen_what_the_melody_may_land_on() {
        let triad = Chord::new(0);
        let seventh = Chord::with(0, Ext::Seventh);
        let ninth = Chord::with(0, Ext::Ninth);

        // Degree 6 is the seventh, degree 1 is the ninth.
        assert!(!triad.contains(6));
        assert!(seventh.contains(6));
        assert!(ninth.contains(6));
        assert!(!triad.contains(1));
        assert!(!seventh.contains(1));
        assert!(ninth.contains(1));

        // Everything still contains its own triad, in any octave.
        for c in [triad, seventh, ninth] {
            for d in [0, 2, 4, 7, 9, 11, -7, -5, -3] {
                assert!(c.contains(d), "{c:?} lost triad tone {d}");
            }
        }
    }

    #[test]
    fn snapping_can_now_reach_the_seventh() {
        // Degree 6 sits a step below the octave. Against a triad the
        // melody gets pushed off it; against a seventh chord it stays.
        assert_ne!(Chord::new(0).snap(6), 6);
        assert_eq!(Chord::with(0, Ext::Seventh).snap(6), 6);
        assert_eq!(Chord::with(0, Ext::Ninth).snap(8), 8);
    }

    #[test]
    fn an_extended_arp_drops_the_root_and_keeps_three_slots() {
        let mode = Mode::Aeolian;
        // A triad arpeggiates from its own root.
        assert_eq!(Chord::new(0).arp(mode)[0], 0);

        // A seventh chord has four tones and three slots, so the root
        // goes: the bass has it, and the third and seventh are what say
        // "seventh chord".
        let seventh = Chord::with(0, Ext::Seventh).arp(mode);
        assert_ne!(seventh[0], 0, "the root should have been dropped");
        // i7 in Aeolian: minor third, fifth, minor seventh.
        assert_eq!(seventh, [3, 7, 10]);

        // A ninth is voiced 3rd/5th/7th/9th, so its three slots are the
        // top three of that.
        let ninth = Chord::with(0, Ext::Ninth).arp(mode);
        assert_eq!(ninth, [7, 10, 14]);
    }

    #[test]
    fn a_secondary_dominant_borrows_its_third_and_its_seventh() {
        // The two chords the maru-sa progression needs, spelled in A
        // minor: the V7 that Aeolian would otherwise write as Em7, and
        // the bIII7 whose flat seventh is a semitone outside the key.
        let v7 = Chord::dom(4, Ext::Seventh);
        // Root, major third, fifth, flat seventh — E G# B D.
        assert_eq!(v7.arp(Mode::Aeolian), [4, 7, 10]);
        assert_eq!(v7.pitch(Mode::Aeolian, 4), 7);
        assert_eq!(v7.pitch(Mode::Aeolian, 6), 11, "the third stayed minor");
        // Aeolian already spells this seventh flat, so nothing moves.
        assert_eq!(v7.pitch(Mode::Aeolian, 10), 17);
        // A diatonic v7 is the thing being fixed: same degrees, no pull.
        assert_eq!(Chord::with(4, Ext::Seventh).arp(Mode::Aeolian), [3, 7, 10]);

        // C7 in A minor: the seventh is Bb, which Aeolian spells as B.
        let flat_three = Chord::dom(2, Ext::Seventh);
        assert_eq!(flat_three.arp(Mode::Aeolian), [4, 7, 10]);
        assert_eq!(flat_three.pitch(Mode::Aeolian, 8), 13);

        // The colouring follows the degree into any octave.
        for oct in [-14, -7, 0, 7, 14] {
            assert_eq!(v7.pitch(Mode::Aeolian, 6 + oct), 11 + oct / 7 * 12);
        }
    }

    #[test]
    fn colouring_leaves_every_other_chord_exactly_as_it_was() {
        // The whole model rests on `Chord::pitch` being `Mode::pitch` for
        // anything that has not explicitly borrowed a third.
        for mode in [Mode::Ionian, Mode::Aeolian, Mode::Dorian, Mode::Phrygian] {
            for degree in 0..7 {
                for ext in [Ext::Triad, Ext::Seventh, Ext::Ninth] {
                    let c = Chord::with(degree, ext);
                    for d in -7..14 {
                        assert_eq!(c.pitch(mode, d), mode.pitch(d));
                    }
                }
                // A dominant triad borrows a third but has no seventh to
                // flatten, so the degree below the octave is left alone.
                let triad = Chord::dom(degree, Ext::Triad);
                assert_eq!(
                    triad.pitch(mode, degree + 6),
                    mode.pitch(degree + 6),
                    "a triad flattened a seventh it does not have"
                );
            }
        }
    }

    #[test]
    fn ladders_are_the_right_length() {
        assert_eq!(Chord::new(0).ladder().1, 3);
        assert_eq!(Chord::with(0, Ext::Seventh).ladder().1, 4);
        assert_eq!(Chord::with(0, Ext::Ninth).ladder().1, 4);
        // Every entry the count claims must be a chord tone.
        for c in [
            Chord::new(2),
            Chord::with(2, Ext::Seventh),
            Chord::with(2, Ext::Ninth),
        ] {
            let (tones, n) = c.ladder();
            for t in &tones[..n] {
                assert!(c.contains(*t), "{c:?} ladder tone {t} is not in the chord");
            }
        }
    }
}
