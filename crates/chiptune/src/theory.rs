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
            Mode::PhrygianDominant => "Phrygian dominant",
        }
    }
}

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A chord as a scale degree of the current mode (0 = I, 4 = V, ...).
/// We never spell chords absolutely: `degree` plus the mode is enough.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub degree: i32,
    pub seventh: bool,
}

impl Chord {
    pub const fn new(degree: i32) -> Self {
        Chord {
            degree,
            seventh: false,
        }
    }

    /// Scale degrees of the chord tones, relative to the tonic.
    pub fn tones(&self) -> [i32; 3] {
        [self.degree, self.degree + 2, self.degree + 4]
    }

    /// True if `degree` (any octave) is a chord tone.
    pub fn contains(&self, degree: i32) -> bool {
        let d = (degree - self.degree).rem_euclid(7);
        d == 0 || d == 2 || d == 4 || (self.seventh && d == 6)
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

    /// Semitone offsets of the triad above its own root, for the fast
    /// arpeggio macro. Read through the mode, so a i chord really is
    /// minor and a V really is major.
    pub fn arp(&self, mode: Mode) -> [i8; 3] {
        let root = mode.pitch(self.degree);
        let tones = self.tones();
        [
            0,
            (mode.pitch(tones[1]) - root) as i8,
            (mode.pitch(tones[2]) - root) as i8,
        ]
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
