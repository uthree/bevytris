//! A dependency-free chiptune engine: a NES-flavoured four-channel
//! synthesizer ([`synth`]) driven by an algorithmic composer ([`compose`]).
//!
//! Nothing in here knows about Bevy — the game wires it up in
//! `src/music.rs`, and `cargo run -p bevytris-chiptune --example render`
//! renders the exact same code to a WAV file offline, which is how the
//! sound gets tuned without launching the game.
//!
//! The design follows the NES 2A03 discipline on purpose: four fixed
//! voices, 4-bit volumes, an 11-bit period table, per-frame (60 Hz) macro
//! envelopes and the console's own nonlinear mixer. Constraint is what
//! makes it sound like a chiptune rather than like a cheap synth.

pub mod compose;
pub mod rng;
pub mod synth;
pub mod theory;
pub mod wav;

/// 48 kHz divides evenly by both 60 (the macro tick) and 240, so the
/// frame clock needs no fractional accumulator.
pub const SAMPLE_RATE: u32 = 48_000;
/// Tracker-style macro rate: volume/duty/arpeggio advance one step here.
pub const FRAME_RATE: u32 = 60;
pub const SAMPLES_PER_FRAME: u32 = SAMPLE_RATE / FRAME_RATE;
/// Sixteenth notes per 4/4 bar — the sequencer's resolution.
pub const STEPS_PER_BAR: u32 = 16;

/// The four hardware voices, in mix order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Voice {
    /// Pulse 1 — melody.
    Lead = 0,
    /// Pulse 2 — harmony, chord bed, arpeggios.
    Harmony = 1,
    /// Triangle — bass (and the body of a kick drum).
    Bass = 2,
    /// Noise — percussion.
    Perc = 3,
}

/// Instrument presets. Each one is a set of macro tables (below); the
/// composer picks by role, never by number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inst {
    /// Short plucked lead, 25% duty — the default melody voice.
    Pluck,
    /// Sustained lead with vibrato, 25% duty.
    Sustain,
    /// Mellow 50%-duty lead for the calm profile.
    Soft,
    /// Held 50%-duty chord bed.
    Organ,
    /// Percussive 12.5%-duty chord stab.
    Stab,
    /// Quiet held pad, sits under everything.
    Pad,
    /// Triangle bass (gate only — the triangle has no volume control).
    Bass,
    Kick,
    Snare,
    Hat,
}

impl Inst {
    /// Which hardware channel this instrument must be played on.
    pub fn voice(self) -> Voice {
        match self {
            Inst::Pluck | Inst::Sustain | Inst::Soft => Voice::Lead,
            Inst::Organ | Inst::Stab | Inst::Pad => Voice::Harmony,
            Inst::Bass => Voice::Bass,
            Inst::Kick | Inst::Snare | Inst::Hat => Voice::Perc,
        }
    }
}

/// One scheduled note. Timestamps are absolute sample counts, so tempo
/// never has to cross the thread boundary into the audio callback.
#[derive(Clone, Copy, Debug)]
pub struct NoteEvent {
    pub at: u64,
    pub inst: Inst,
    /// MIDI note number. Meaningless for the drum instruments.
    pub midi: u8,
    /// 0-127, scales the macro volume.
    pub vel: u8,
    /// Note length in 60 Hz frames.
    pub frames: u16,
    /// Semitone offsets cycled once per frame — the classic chiptune
    /// "chord on a monophonic channel" trick. `None` for plain notes.
    pub arp: Option<[i8; 3]>,
}

impl NoteEvent {
    pub fn voice(&self) -> Voice {
        self.inst.voice()
    }
}

// ---------------------------------------------------------------------------
// Instrument macro tables
// ---------------------------------------------------------------------------

/// A tracker macro: one value per 60 Hz frame, optionally looping.
pub struct Macro {
    pub v: &'static [i8],
    pub loop_at: Option<usize>,
}

impl Macro {
    /// Value at `cursor`, following the loop point; `None` once a
    /// non-looping macro has run out (the note is then finished).
    pub fn at(&self, cursor: usize) -> Option<i8> {
        if cursor < self.v.len() {
            return Some(self.v[cursor]);
        }
        let lp = self.loop_at?;
        let span = self.v.len() - lp;
        Some(self.v[lp + (cursor - self.v.len()) % span])
    }
}

pub struct InstDef {
    /// 0-15, the NES 4-bit volume range.
    pub vol: Macro,
    /// Pulse duty index: 0 = 12.5%, 1 = 25%, 2 = 50%.
    pub duty: u8,
    /// Frames before vibrato starts; 0 depth disables it. The delay is
    /// what makes vibrato a property of *long* notes without the composer
    /// having to pick a different instrument for them.
    pub vib_delay: u16,
    pub vib_depth: f32,
    pub vib_rate: f32,
    /// Semitone scale of the pitch fall at the tail of a held note. 0
    /// disables it; see [`synth`] for the length threshold.
    pub fall: f32,
    /// Index into [`synth::NOISE_PERIODS`] for the drum instruments.
    pub noise_idx: u8,
    pub noise_short: bool,
}

const fn m(v: &'static [i8], loop_at: Option<usize>) -> Macro {
    Macro { v, loop_at }
}

/// No vibrato, no fall — the shared tail for the instruments that hold
/// steady.
const STEADY: (u16, f32, f32, f32) = (0, 0.0, 0.0, 0.0);

const PLUCK: InstDef = InstDef {
    vol: m(&[15, 15, 14, 12, 11, 10, 10, 9, 9, 8, 8, 7], Some(8)),
    duty: 1,
    // Short notes finish before the delay expires, so only held ones
    // wobble — one instrument covers both jobs.
    vib_delay: 13,
    vib_depth: 0.14,
    vib_rate: 6.5,
    fall: 0.55,
    noise_idx: 0,
    noise_short: false,
};
const SUSTAIN: InstDef = InstDef {
    vol: m(&[11, 14, 14, 13, 13, 12, 12, 12], Some(4)),
    duty: 1,
    vib_delay: 12,
    vib_depth: 0.17,
    vib_rate: 6.0,
    fall: 0.65,
    noise_idx: 0,
    noise_short: false,
};
const SOFT: InstDef = InstDef {
    vol: m(&[7, 10, 12, 12, 11, 11, 10, 10, 9], Some(5)),
    duty: 2,
    vib_delay: 18,
    vib_depth: 0.11,
    vib_rate: 5.0,
    fall: 0.34,
    noise_idx: 0,
    noise_short: false,
};
const ORGAN: InstDef = InstDef {
    vol: m(&[10, 12, 11, 11, 10, 10], Some(2)),
    duty: 2,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    // A chord bed that slides out of tune under the melody just sounds
    // broken, so the harmony voices never fall.
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
};
const STAB: InstDef = InstDef {
    vol: m(&[15, 13, 10, 8, 6, 4, 3, 2, 1], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
};
const PAD: InstDef = InstDef {
    vol: m(&[4, 6, 7, 8, 8, 7, 7], Some(4)),
    duty: 2,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
};
const BASS: InstDef = InstDef {
    // The triangle has no volume register: this macro is a pure gate.
    vol: m(&[15], Some(0)),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    // The low end stays solid.
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
};
const KICK: InstDef = InstDef {
    vol: m(&[13, 9, 5, 2, 1], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 13,
    noise_short: false,
};
const SNARE: InstDef = InstDef {
    vol: m(&[15, 13, 11, 9, 7, 5, 4, 3, 2, 1], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 9,
    noise_short: false,
};
const HAT: InstDef = InstDef {
    vol: m(&[9, 5, 3, 1], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 2,
    noise_short: false,
};

pub fn inst_def(i: Inst) -> &'static InstDef {
    match i {
        Inst::Pluck => &PLUCK,
        Inst::Sustain => &SUSTAIN,
        Inst::Soft => &SOFT,
        Inst::Organ => &ORGAN,
        Inst::Stab => &STAB,
        Inst::Pad => &PAD,
        Inst::Bass => &BASS,
        Inst::Kick => &KICK,
        Inst::Snare => &SNARE,
        Inst::Hat => &HAT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_loops_at_its_loop_point() {
        let mac = m(&[9, 8, 7, 6], Some(2));
        assert_eq!(mac.at(0), Some(9));
        assert_eq!(mac.at(3), Some(6));
        // 4 -> back to index 2, then alternating 7, 6, 7, 6 ...
        assert_eq!(mac.at(4), Some(7));
        assert_eq!(mac.at(5), Some(6));
        assert_eq!(mac.at(6), Some(7));
    }

    #[test]
    fn one_shot_macro_ends() {
        let mac = m(&[15, 7], None);
        assert_eq!(mac.at(1), Some(7));
        assert_eq!(mac.at(2), None);
    }

    #[test]
    fn instruments_land_on_their_own_channel() {
        assert_eq!(Inst::Pluck.voice(), Voice::Lead);
        assert_eq!(Inst::Stab.voice(), Voice::Harmony);
        assert_eq!(Inst::Bass.voice(), Voice::Bass);
        assert_eq!(Inst::Hat.voice(), Voice::Perc);
    }
}
