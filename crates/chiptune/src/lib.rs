//! A dependency-free chiptune engine: an eight-voice synthesizer
//! ([`synth`]) driven by an algorithmic composer ([`compose`]).
//!
//! Nothing in here knows about Bevy — the game wires it up in
//! `src/music.rs`, and `cargo run -p bevytris-chiptune --example render`
//! renders the exact same code to a WAV file offline, which is how the
//! sound gets tuned without launching the game.
//!
//! The design follows the NES 2A03 discipline on purpose — 4-bit volumes,
//! an 11-bit period table, per-frame (60 Hz) macro envelopes, the
//! console's own nonlinear mixer — and then adds what cartridges added:
//! a third pulse and a sawtooth after the VRC6, a user-defined wavetable
//! after the Game Boy, and a second noise channel so hats stop being
//! swallowed by kicks. Constraint is what makes it sound like a chiptune
//! rather than like a cheap synth; the extra voices widen the arrangement
//! without loosening any of the rules.

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

/// The eight hardware voices, in mix order.
///
/// The first four are the Famicom's own channels and go through its
/// nonlinear mixer. The rest stand in for the expansion chips cartridges
/// used to add — a third pulse and a sawtooth in the manner of Konami's
/// VRC6, and a user-defined wavetable in the manner of the Game Boy —
/// and, like real expansion audio, mix in linearly on top.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Voice {
    /// Pulse 1 — melody.
    Lead = 0,
    /// Pulse 2 — harmony, chord bed, arpeggios.
    Harmony = 1,
    /// Pulse 3 — counter-melody, echoes, fast arpeggios.
    Counter = 2,
    /// Sawtooth — a buzzier lead or a bass double.
    Saw = 3,
    /// 32-step wavetable — pads and bells.
    Wave = 4,
    /// Triangle — bass (and the body of a kick drum).
    Bass = 5,
    /// Noise — kick and snare.
    Perc = 6,
    /// Second noise channel — hats and shakers, so they stop being
    /// swallowed every time the kick lands.
    Hat = 7,
}

pub const VOICE_COUNT: usize = 8;

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
    /// Quiet delayed copy of the melody, on the third pulse.
    Echo,
    /// Fast chord arpeggio in sixteenths.
    Arp,
    /// Buzzy sawtooth lead.
    SawLead,
    /// Sawtooth doubling the bass an octave up, for weight.
    SawBass,
    /// Struck wavetable bell — bright odd harmonics, long decay.
    Bell,
    /// Soft sine wavetable.
    Glass,
    /// Drawbar-ish wavetable pad.
    WaveOrgan,
    /// Round wavetable bass for the calm profiles.
    WaveBass,
    /// Triangle bass (gate only — the triangle has no volume control).
    Bass,
    Kick,
    Snare,
    Hat,
    /// Sixteenth-note shaker, quieter and higher than the hat.
    Shaker,
    /// Long ringing noise hit, for the downbeat of the final chorus.
    Crash,
}

impl Inst {
    /// Which hardware channel this instrument must be played on.
    pub fn voice(self) -> Voice {
        match self {
            Inst::Pluck | Inst::Sustain | Inst::Soft => Voice::Lead,
            Inst::Organ | Inst::Stab | Inst::Pad => Voice::Harmony,
            Inst::Echo | Inst::Arp => Voice::Counter,
            Inst::SawLead | Inst::SawBass => Voice::Saw,
            Inst::Bell | Inst::Glass | Inst::WaveOrgan | Inst::WaveBass => Voice::Wave,
            Inst::Bass => Voice::Bass,
            Inst::Kick | Inst::Snare => Voice::Perc,
            Inst::Hat | Inst::Shaker | Inst::Crash => Voice::Hat,
        }
    }

    /// True for the unpitched drum instruments.
    pub fn is_drum(self) -> bool {
        matches!(self.voice(), Voice::Perc | Voice::Hat)
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
    /// Which 32-step table the wavetable channel plays.
    pub wave: u8,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
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
    wave: 0,
};

// --- expansion voices ------------------------------------------------------

const ECHO: InstDef = InstDef {
    vol: m(&[7, 6, 6, 5, 5, 4, 4, 3, 3, 2, 2, 1], None),
    // Thin duty so the echo reads as a reflection rather than as a second
    // melody competing with the lead.
    duty: 0,
    vib_delay: 16,
    vib_depth: 0.12,
    vib_rate: 5.5,
    fall: 0.4,
    noise_idx: 0,
    noise_short: false,
    wave: 0,
};
const ARP: InstDef = InstDef {
    vol: m(&[10, 8, 6, 5], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
    wave: 0,
};
const SAW_LEAD: InstDef = InstDef {
    vol: m(&[13, 14, 13, 12, 12, 11, 11, 10], Some(4)),
    duty: 0,
    vib_delay: 14,
    vib_depth: 0.15,
    vib_rate: 5.8,
    fall: 0.6,
    noise_idx: 0,
    noise_short: false,
    wave: 0,
};
const SAW_BASS: InstDef = InstDef {
    vol: m(&[12, 11, 10, 10, 9, 9], Some(3)),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
    wave: 0,
};
const BELL: InstDef = InstDef {
    vol: m(&[15, 13, 11, 10, 9, 8, 7, 6, 5, 4, 4, 3, 2, 2, 1, 1], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
    wave: 2,
};
const GLASS: InstDef = InstDef {
    vol: m(&[9, 11, 11, 10, 10, 9, 9], Some(4)),
    duty: 0,
    vib_delay: 22,
    vib_depth: 0.09,
    vib_rate: 4.5,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
    wave: 0,
};
const WAVE_ORGAN: InstDef = InstDef {
    vol: m(&[7, 9, 10, 10, 9, 9], Some(3)),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
    wave: 1,
};
const WAVE_BASS: InstDef = InstDef {
    vol: m(&[12, 11, 11, 10, 10], Some(2)),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 0,
    noise_short: false,
    wave: 3,
};
const SHAKER: InstDef = InstDef {
    vol: m(&[6, 3, 1], None),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 1,
    noise_short: false,
    wave: 0,
};

const CRASH: InstDef = InstDef {
    vol: m(
        &[15, 15, 14, 13, 12, 11, 10, 9, 9, 8, 7, 7, 6, 5, 5, 4, 4, 3, 3, 2, 2, 1, 1],
        None,
    ),
    duty: 0,
    vib_delay: STEADY.0,
    vib_depth: STEADY.1,
    vib_rate: STEADY.2,
    fall: STEADY.3,
    noise_idx: 3,
    noise_short: false,
    wave: 0,
};

pub fn inst_def(i: Inst) -> &'static InstDef {
    match i {
        Inst::Pluck => &PLUCK,
        Inst::Sustain => &SUSTAIN,
        Inst::Soft => &SOFT,
        Inst::Organ => &ORGAN,
        Inst::Stab => &STAB,
        Inst::Pad => &PAD,
        Inst::Echo => &ECHO,
        Inst::Arp => &ARP,
        Inst::SawLead => &SAW_LEAD,
        Inst::SawBass => &SAW_BASS,
        Inst::Bell => &BELL,
        Inst::Glass => &GLASS,
        Inst::WaveOrgan => &WAVE_ORGAN,
        Inst::WaveBass => &WAVE_BASS,
        Inst::Bass => &BASS,
        Inst::Kick => &KICK,
        Inst::Snare => &SNARE,
        Inst::Hat => &HAT,
        Inst::Shaker => &SHAKER,
        Inst::Crash => &CRASH,
    }
}

/// Every instrument, for tests and for the offline renderer.
pub const ALL_INSTRUMENTS: [Inst; 20] = [
    Inst::Pluck,
    Inst::Sustain,
    Inst::Soft,
    Inst::Organ,
    Inst::Stab,
    Inst::Pad,
    Inst::Echo,
    Inst::Arp,
    Inst::SawLead,
    Inst::SawBass,
    Inst::Bell,
    Inst::Glass,
    Inst::WaveOrgan,
    Inst::WaveBass,
    Inst::Bass,
    Inst::Kick,
    Inst::Snare,
    Inst::Hat,
    Inst::Shaker,
    Inst::Crash,
];

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
        assert_eq!(Inst::Echo.voice(), Voice::Counter);
        assert_eq!(Inst::SawBass.voice(), Voice::Saw);
        assert_eq!(Inst::Bell.voice(), Voice::Wave);
        assert_eq!(Inst::Bass.voice(), Voice::Bass);
        assert_eq!(Inst::Kick.voice(), Voice::Perc);
        // Hats live on their own noise channel, so a kick no longer eats
        // them.
        assert_eq!(Inst::Hat.voice(), Voice::Hat);
        assert_ne!(Inst::Kick.voice(), Inst::Hat.voice());
    }

    #[test]
    fn every_instrument_is_defined_and_listed() {
        assert_eq!(ALL_INSTRUMENTS.len(), 20);
        for i in ALL_INSTRUMENTS {
            let d = inst_def(i);
            assert!(!d.vol.v.is_empty(), "{i:?} has no volume macro");
            assert!(d.duty <= 2, "{i:?} has an out-of-range duty");
            assert!(d.wave < 4, "{i:?} points at a missing wavetable");
            assert!(d.noise_idx < 16);
            assert_eq!(
                i.is_drum(),
                matches!(
                    i,
                    Inst::Kick | Inst::Snare | Inst::Hat | Inst::Shaker | Inst::Crash
                )
            );
        }
        // Every voice must have at least one instrument that can reach it.
        for v in 0..VOICE_COUNT {
            assert!(
                ALL_INSTRUMENTS.iter().any(|i| i.voice() as usize == v),
                "voice {v} is unreachable"
            );
        }
    }
}
