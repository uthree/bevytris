//! The synthesizer: eight voices — three pulses, a sawtooth, a 32-step
//! wavetable, a triangle and two noise channels — mixed through the NES's
//! own nonlinear mixer (for the four the console actually had), placed in
//! a stereo image, and run through the master filter chain.
//!
//! Three deliberate choices are worth knowing about before touching this:
//!
//! * **Pitch is quantized to the hardware's 11-bit period register.** The
//!   slight detuning of high notes that falls out of this is a large part
//!   of why NES music sounds like NES music, so it is a feature. Vibrato
//!   is applied to the period, not the frequency, for the same reason.
//! * **The pulse channels are band-limited with PolyBLEP.** Naive squares
//!   are not the retro aesthetic, they are a defect: a 12.5%-duty note up
//!   near C7 throws about eighteen inharmonic images below 1 kHz, and they
//!   move erratically as the melody moves. That is the "sour chiptune"
//!   sound. Nine lines of correction removes it. The lo-fi character comes
//!   from the 4-bit volumes, the period quantization, the 32-step triangle
//!   staircase and the nonlinear mixer instead.
//! * **Panning belongs to the channel, not the note.** That is the tracker
//!   convention, and it keeps the image stable instead of swimming about
//!   as the arrangement changes. See [`VOICE_PAN`].

use crate::{
    FRAME_RATE, Inst, InstDef, NoteEvent, SAMPLES_PER_FRAME, SAMPLE_RATE, VOICE_COUNT, Voice,
    inst_def,
};

/// Where each voice sits in the stereo image, -1 (hard left) to +1.
///
/// Tracker convention: panning is a property of the *channel*, not of the
/// note, so the image stays stable instead of swimming about. The two
/// rules that matter are that the low end stays dead centre — a panned
/// bass falls apart on speakers — and that the echo sits opposite the
/// lead it is echoing, which is what turns it from a doubling into space.
const VOICE_PAN: [f32; VOICE_COUNT] = [
    0.22,  // Lead
    -0.42, // Harmony
    0.55,  // Counter
    -0.30, // Saw
    0.14,  // Wave
    0.0,   // Bass
    -0.08, // Perc
    0.38,  // Hat
    -0.05, // Sample — drum bodies and impacts, so near enough centred
];

/// NTSC 2A03 clock. Every period table below derives from it.
const CPU_HZ: f32 = 1_789_773.0;
const CYCLES_PER_SAMPLE: f32 = CPU_HZ / SAMPLE_RATE as f32;

/// The triangle channel's literal 32-entry 4-bit sequence. The doubled 0
/// and 15 at the turning points are part of the sound; do not replace this
/// with an analytic triangle.
const TRIANGLE_TABLE: [f32; 32] = [
    15.0, 14.0, 13.0, 12.0, 11.0, 10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 1.0,
    2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
];

/// Duty cycle fractions for the three distinct pulse timbres. (The
/// hardware's fourth setting, 75%, is 25% phase-inverted and sounds
/// identical, so there is no point exposing it.)
const DUTIES: [f32; 3] = [0.125, 0.25, 0.5];

/// NTSC noise period table, in CPU cycles between LFSR clocks. Low indices
/// are hats, the middle is snares, the top is kicks and rumbles.
pub const NOISE_PERIODS: [f32; 16] = [
    4.0, 8.0, 16.0, 32.0, 64.0, 96.0, 128.0, 160.0, 202.0, 254.0, 380.0, 508.0, 762.0, 1016.0,
    2034.0, 4068.0,
];

/// Equal-power pan gains for a position in -1..1, narrowed by `width`
/// (1 = full stereo, 0 = mono).
fn pan_gains(pan: f32, width: f32) -> (f32, f32) {
    let p = (pan * width).clamp(-1.0, 1.0);
    let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
    (a.cos(), a.sin())
}

/// The wavetable channel's four 32-step, 4-bit tables. Built at runtime
/// rather than written out as literals so the harmonic recipes stay
/// readable.
fn build_wavetables() -> [[f32; 32]; 4] {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 15.0).round();
    let mut t = [[0.0f32; 32]; 4];
    for i in 0..32 {
        let th = std::f32::consts::TAU * i as f32 / 32.0;
        // Pure sine — the softest thing the chip can say.
        t[0][i] = q(0.5 + 0.5 * th.sin());
        // Drawbar organ: fundamental, octave, twelfth.
        t[1][i] =
            q(0.5 + 0.5 * (0.62 * th.sin() + 0.26 * (2.0 * th).sin() + 0.12 * (3.0 * th).sin()));
        // Bell: odd harmonics only, which is what makes it read as struck
        // metal rather than as a blown pipe.
        t[2][i] =
            q(0.5 + 0.5 * (0.55 * th.sin() + 0.28 * (3.0 * th).sin() + 0.17 * (5.0 * th).sin()));
        // Reed: a rounded saw, for weight without buzz.
        t[3][i] = q(0.5
            + 0.5
                * (0.50 * th.sin()
                    + 0.25 * (2.0 * th).sin()
                    + 0.15 * (3.0 * th).sin()
                    + 0.10 * (4.0 * th).sin()));
    }
    t
}

/// The synthesized sample bank — the one place the engine stops obeying
/// the chip. These are built at startup from plain maths, so they cost no
/// asset files, and they do the things four-bit oscillators cannot: a
/// drum with an actual body, a sweep, a sub-bass impact.
fn build_samples() -> Vec<Vec<f32>> {
    let sr = SAMPLE_RATE as f32;
    let tau = std::f32::consts::TAU;
    // A local LCG so the bank is byte-identical on every run.
    let mut seed = 0x1234_5678u32;
    let noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
    };

    let render = |secs: f32, mut f: Box<dyn FnMut(f32, usize) -> f32>| -> Vec<f32> {
        let n = (sr * secs) as usize;
        let mut buf: Vec<f32> = (0..n).map(|i| f(i as f32 / sr, i)).collect();
        let peak = buf.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        if peak > 1e-6 {
            for s in &mut buf {
                *s /= peak;
            }
        }
        buf
    };

    let mut out = Vec::new();

    // 0 — kick: a sine whose pitch collapses, plus a click for the beater.
    let mut phase = 0.0f32;
    out.push(render(
        0.38,
        Box::new(move |t, i| {
            let f = 42.0 + 96.0 * (-t * 34.0).exp();
            phase += tau * f / 48_000.0;
            let click = if i < 90 {
                (1.0 - i as f32 / 90.0).powi(2) * 0.55
            } else {
                0.0
            };
            phase.sin() * (-t * 8.5).exp() + click
        }),
    ));

    // 1 — snare: noise over two tuned tones, the way a real shell rings.
    let (mut p1, mut p2) = (0.0f32, 0.0f32);
    let mut n1 = noise;
    out.push(render(
        0.24,
        Box::new(move |t, _| {
            p1 += tau * 188.0 / 48_000.0;
            p2 += tau * 331.0 / 48_000.0;
            n1() * (-t * 21.0).exp() + p1.sin() * (-t * 16.0).exp() * 0.5
                + p2.sin() * (-t * 19.0).exp() * 0.3
        }),
    ));

    // 2 — clap: four bursts a few milliseconds apart, then a short room.
    let mut n2 = noise;
    let mut hp = 0.0f32;
    out.push(render(
        0.30,
        Box::new(move |t, _| {
            let burst = [0.000f32, 0.009, 0.018, 0.027]
                .iter()
                .map(|&s| {
                    if t >= s {
                        (-(t - s) * 320.0).exp()
                    } else {
                        0.0
                    }
                })
                .sum::<f32>();
            let tail = if t > 0.03 { (-(t - 0.03) * 26.0).exp() } else { 0.0 };
            let raw = n2() * (burst + tail * 0.5);
            // One-pole high-pass, so it snaps instead of thudding.
            hp += (raw - hp) * 0.12;
            raw - hp
        }),
    ));

    // 3 — tom.
    let mut p3 = 0.0f32;
    out.push(render(
        0.34,
        Box::new(move |t, _| {
            let f = 92.0 + 110.0 * (-t * 16.0).exp();
            p3 += tau * f / 48_000.0;
            p3.sin() * (-t * 10.0).exp()
        }),
    ));

    // 4 — riser: the sweep that announces a chorus. A sine climbing three
    // and a half octaves under brightening noise, swelling the whole way
    // and cut off dead at the top so the downbeat lands in the gap.
    let mut p4 = 0.0f32;
    let mut n4 = noise;
    let mut lp = 0.0f32;
    out.push(render(
        1.5,
        Box::new(move |t, _| {
            let k = (t / 1.5).min(1.0);
            let f = 260.0 * (11.0f32).powf(k);
            p4 += tau * f / 48_000.0;
            let raw = n4();
            lp += (raw - lp) * (0.02 + 0.75 * k);
            let swell = k.powf(1.6);
            // Hard stop at the very end: the silence is the point.
            let cut = if k > 0.97 { (1.0 - k) / 0.03 } else { 1.0 };
            (p4.sin() * 0.55 + lp * 0.85) * swell * cut
        }),
    ));

    // 5 — impact: the sub-bass drop the riser was pointing at.
    let mut p5 = 0.0f32;
    let mut n5 = noise;
    out.push(render(
        1.1,
        Box::new(move |t, i| {
            let f = 29.0 + 52.0 * (-t * 12.0).exp();
            p5 += tau * f / 48_000.0;
            let crack = if i < 1400 {
                n5() * (1.0 - i as f32 / 1400.0).powi(3) * 0.5
            } else {
                0.0
            };
            p5.sin() * (-t * 3.4).exp() + crack
        }),
    ));

    out
}

/// PolyBLEP: a two-sample polynomial correction around a step
/// discontinuity, which cancels most of the aliasing a naive square
/// generates.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

fn midi_to_hz(midi: f32) -> f32 {
    440.0 * ((midi - 69.0) / 12.0).exp2()
}

/// Snap a frequency to what the pulse channels' 11-bit period register can
/// actually produce.
fn quantize_pulse(hz: f32) -> f32 {
    let period = (CPU_HZ / (16.0 * hz.max(1.0)) - 1.0).round();
    let period = period.clamp(8.0, 2047.0);
    CPU_HZ / (16.0 * (period + 1.0))
}

/// Same for the triangle, which counts at half the rate — which is why
/// the bass lives there: the pulse channels cannot reach below about A1.
fn quantize_triangle(hz: f32) -> f32 {
    let period = (CPU_HZ / (32.0 * hz.max(1.0)) - 1.0).round();
    let period = period.clamp(2.0, 2047.0);
    CPU_HZ / (32.0 * (period + 1.0))
}

fn vel_scale(vel: u8) -> f32 {
    0.35 + 0.65 * (vel.min(127) as f32 / 127.0)
}

/// A note must last this many frames (~0.4 s) before it gets a tail fall.
/// Anything shorter reads as a wrong note rather than as expression.
const FALL_MIN_FRAMES: u16 = 24;
/// How many frames the fall occupies at the end of the note.
const FALL_FRAMES: u16 = 6;

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct OnePoleLp {
    y: f32,
    a: f32,
}

impl OnePoleLp {
    fn new(cutoff: f32) -> Self {
        let mut f = OnePoleLp { y: 0.0, a: 0.0 };
        f.set_cutoff(cutoff);
        f
    }
    fn set_cutoff(&mut self, hz: f32) {
        let hz = hz.clamp(20.0, SAMPLE_RATE as f32 * 0.45);
        self.a = 1.0 - (-std::f32::consts::TAU * hz / SAMPLE_RATE as f32).exp();
    }
    fn step(&mut self, x: f32) -> f32 {
        self.y += (x - self.y) * self.a;
        self.y
    }
}

#[derive(Clone, Copy)]
struct OnePoleHp {
    y: f32,
    x1: f32,
    a: f32,
}

impl OnePoleHp {
    fn new(cutoff: f32) -> Self {
        let rc = 1.0 / (std::f32::consts::TAU * cutoff);
        let dt = 1.0 / SAMPLE_RATE as f32;
        OnePoleHp {
            y: 0.0,
            x1: 0.0,
            a: rc / (rc + dt),
        }
    }
    fn step(&mut self, x: f32) -> f32 {
        self.y = self.a * (self.y + x - self.x1);
        self.x1 = x;
        self.y
    }
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

/// Shared per-note macro playback state.
struct VoiceState {
    inst: &'static InstDef,
    which: Inst,
    active: bool,
    cursor: usize,
    frames_left: u16,
    frames_played: u16,
    vel: f32,
    arp: Option<[i8; 3]>,
    base_midi: f32,
    /// The note's full length, so the tail fall knows where the tail is.
    total_frames: u16,
    /// Glissando: where the slide starts, and how much of it is left.
    glide_from: f32,
    glide_left: u16,
    glide_total: u16,
    /// Volume is ramped per sample between macro steps: the raw 60 Hz
    /// staircase leaves an error signal only ~14 dB below the tone, which
    /// is an audible buzz on short decays.
    gain: f32,
    gain_step: f32,
}

impl VoiceState {
    fn silent() -> Self {
        VoiceState {
            inst: inst_def(Inst::Pluck),
            which: Inst::Pluck,
            active: false,
            cursor: 0,
            frames_left: 0,
            frames_played: 0,
            vel: 1.0,
            arp: None,
            base_midi: 60.0,
            total_frames: 0,
            glide_from: 60.0,
            glide_left: 0,
            glide_total: 0,
            gain: 0.0,
            gain_step: 0.0,
        }
    }

    fn start(&mut self, ev: &NoteEvent) {
        // A glissando only means anything if there was a note to slide
        // from — an opening note just attacks.
        if ev.glide > 0 && self.active {
            self.glide_from = self.base_midi;
            self.glide_left = ev.glide as u16;
            self.glide_total = ev.glide as u16;
        } else {
            self.glide_left = 0;
        }
        self.inst = inst_def(ev.inst);
        self.which = ev.inst;
        self.active = true;
        self.cursor = 0;
        self.frames_left = ev.frames.max(1);
        self.total_frames = ev.frames.max(1);
        self.frames_played = 0;
        self.vel = vel_scale(ev.vel);
        self.arp = ev.arp;
        self.base_midi = ev.midi as f32;
    }

    /// Advance one 60 Hz frame; returns the target volume in 0..15.
    fn tick(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        if self.frames_left == 0 {
            self.active = false;
            return 0.0;
        }
        self.frames_left -= 1;
        let Some(v) = self.inst.vol.at(self.cursor) else {
            self.active = false;
            return 0.0;
        };
        self.cursor += 1;
        self.frames_played += 1;
        self.glide_left = self.glide_left.saturating_sub(1);
        v as f32 * self.vel
    }

    /// Current pitch in MIDI numbers, including the arpeggio macro and
    /// vibrato.
    fn midi_now(&self) -> f32 {
        let arp = match self.arp {
            Some(a) => a[self.frames_played.saturating_sub(1) as usize % 3] as f32,
            None => 0.0,
        };
        let mut m = self.base_midi + arp;
        if self.glide_left > 0 && self.glide_total > 0 {
            let k = self.glide_left as f32 / self.glide_total as f32;
            m += (self.glide_from - self.base_midi) * k;
        }
        let d = self.inst;
        if d.vib_depth > 0.0 && self.frames_played > d.vib_delay {
            let t = (self.frames_played - d.vib_delay) as f32 / FRAME_RATE as f32;
            m += d.vib_depth * (std::f32::consts::TAU * d.vib_rate * t).sin();
        }
        // Tail fall: an accelerating slide off the end of a held note.
        // Squaring the progress is what makes it read as a fall rather
        // than as a portamento to nowhere.
        if d.fall > 0.0 && self.total_frames >= FALL_MIN_FRAMES && self.frames_left < FALL_FRAMES {
            let k = (FALL_FRAMES - self.frames_left) as f32;
            m -= d.fall * k * k / 3.0;
        }
        m
    }

    fn set_gain_target(&mut self, target: f32) {
        self.gain_step = (target - self.gain) / SAMPLES_PER_FRAME as f32;
    }
}

struct PulseCh {
    st: VoiceState,
    phase: f32,
    inc: f32,
    duty: f32,
}

impl PulseCh {
    fn new() -> Self {
        PulseCh {
            st: VoiceState::silent(),
            phase: 0.0,
            inc: 0.0,
            duty: 0.5,
        }
    }

    fn tick_frame(&mut self) {
        let target = self.st.tick();
        self.st.set_gain_target(target);
        if self.st.active {
            let hz = quantize_pulse(midi_to_hz(self.st.midi_now()));
            self.inc = hz / SAMPLE_RATE as f32;
            self.duty = DUTIES[(self.st.inst.duty as usize).min(2)];
        }
    }

    /// Amplitude in the hardware's 0..15 range.
    fn sample(&mut self) -> f32 {
        self.st.gain += self.st.gain_step;
        if self.st.gain <= 0.0001 && !self.st.active {
            self.st.gain = 0.0;
            return 0.0;
        }
        let dt = self.inc;
        let t = self.phase;
        let mut v = if t < self.duty { 1.0 } else { -1.0 };
        v += poly_blep(t, dt);
        let mut fall = t + 1.0 - self.duty;
        if fall >= 1.0 {
            fall -= 1.0;
        }
        v -= poly_blep(fall, dt);
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // Map the bipolar wave onto the channel's unipolar 0..15 DAC.
        (self.st.gain * 0.5 * (v + 1.0)).clamp(0.0, 15.0)
    }
}

struct TriCh {
    st: VoiceState,
    phase: f32,
    inc: f32,
    /// Gate envelope. The triangle has no volume control, so notes are
    /// gated — but a hard gate clicks, hence a ~2 ms ramp.
    gate: f32,
    gate_target: f32,
    /// Frames left of a kick drum body stealing the channel.
    kick_frames: u16,
}

impl TriCh {
    fn new() -> Self {
        TriCh {
            st: VoiceState::silent(),
            phase: 0.0,
            inc: 0.0,
            gate: 0.0,
            gate_target: 0.0,
            kick_frames: 0,
        }
    }

    fn kick(&mut self) {
        self.kick_frames = 3;
    }

    fn tick_frame(&mut self) {
        let v = self.st.tick();
        if self.kick_frames > 0 {
            // A short descending sweep under the noise burst: this is how
            // NES games faked a kick drum.
            let step = 3 - self.kick_frames;
            let midi = 43.0 - 7.0 * step as f32;
            self.inc = quantize_triangle(midi_to_hz(midi)) / SAMPLE_RATE as f32;
            self.gate_target = 1.0;
            self.kick_frames -= 1;
            return;
        }
        self.gate_target = if v > 0.0 { 1.0 } else { 0.0 };
        if self.st.active {
            self.inc = quantize_triangle(midi_to_hz(self.st.midi_now())) / SAMPLE_RATE as f32;
        }
    }

    fn sample(&mut self) -> f32 {
        // ~2 ms slew.
        let slew = 1.0 / (SAMPLE_RATE as f32 * 0.002);
        self.gate += (self.gate_target - self.gate).clamp(-slew, slew);
        if self.gate <= 0.0001 {
            return 0.0;
        }
        let idx = (self.phase * 32.0) as usize & 31;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // Scaling the whole staircase (rather than just its swing about
        // 7.5) means a closed gate contributes no DC at all — otherwise
        // an idle triangle parks a constant in the mixer.
        TRIANGLE_TABLE[idx] * self.gate
    }
}

/// VRC6-style sawtooth: an accumulator stepped seven times per period,
/// with the top five bits driving the DAC. The staircase is the point —
/// it is what makes this buzz rather than sing.
struct SawCh {
    st: VoiceState,
    phase: f32,
    inc: f32,
}

impl SawCh {
    fn new() -> Self {
        SawCh {
            st: VoiceState::silent(),
            phase: 0.0,
            inc: 0.0,
        }
    }

    fn tick_frame(&mut self) {
        let target = self.st.tick();
        self.st.set_gain_target(target);
        if self.st.active {
            self.inc = quantize_pulse(midi_to_hz(self.st.midi_now())) / SAMPLE_RATE as f32;
        }
    }

    /// Amplitude in 0..31, the saw's own 5-bit range.
    fn sample(&mut self) -> f32 {
        self.st.gain += self.st.gain_step;
        if self.st.gain <= 0.0001 && !self.st.active {
            self.st.gain = 0.0;
            return 0.0;
        }
        let step = (self.phase * 7.0) as u32 + 1;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // rate 0..36 keeps the eight-bit accumulator from wrapping, which
        // on real hardware turns into a nasty fold-over.
        let rate = self.st.gain * 2.4;
        ((rate * step as f32) / 8.0).clamp(0.0, 31.0).floor()
    }
}

/// Game Boy-style 32-step, 4-bit user wavetable.
struct WaveCh {
    st: VoiceState,
    phase: f32,
    inc: f32,
    table: usize,
}

impl WaveCh {
    fn new() -> Self {
        WaveCh {
            st: VoiceState::silent(),
            phase: 0.0,
            inc: 0.0,
            table: 0,
        }
    }

    fn tick_frame(&mut self) {
        let target = self.st.tick();
        self.st.set_gain_target(target);
        if self.st.active {
            self.inc = quantize_pulse(midi_to_hz(self.st.midi_now())) / SAMPLE_RATE as f32;
            self.table = (self.st.inst.wave as usize).min(3);
        }
    }

    fn sample(&mut self, tables: &[[f32; 32]; 4]) -> f32 {
        self.st.gain += self.st.gain_step;
        if self.st.gain <= 0.0001 && !self.st.active {
            self.st.gain = 0.0;
            return 0.0;
        }
        let idx = (self.phase * 32.0) as usize & 31;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // The table is unipolar 0..15; scaling by the envelope keeps the
        // DC it carries proportional, and the master high-pass removes it.
        tables[self.table][idx] * (self.st.gain / 15.0)
    }
}

/// Sample playback, after the NES's DPCM channel. Two slots rather than
/// one: a riser lasts a second and a half, and a kick landing under it
/// must not cut it off.
struct SampleCh {
    slots: [SampleSlot; 2],
    next: usize,
}

#[derive(Clone, Copy, Default)]
struct SampleSlot {
    bank: usize,
    pos: f32,
    rate: f32,
    gain: f32,
    active: bool,
}

impl SampleCh {
    fn new() -> Self {
        SampleCh {
            slots: [SampleSlot::default(); 2],
            next: 0,
        }
    }

    fn start(&mut self, ev: &NoteEvent) {
        let slot = &mut self.slots[self.next];
        self.next = (self.next + 1) % 2;
        slot.bank = ev.inst.sample();
        slot.pos = 0.0;
        slot.rate = 1.0;
        slot.gain = vel_scale(ev.vel);
        slot.active = true;
    }

    fn stop(&mut self) {
        for s in &mut self.slots {
            s.active = false;
        }
    }

    fn sample(&mut self, bank: &[Vec<f32>]) -> f32 {
        let mut out = 0.0;
        for s in &mut self.slots {
            if !s.active {
                continue;
            }
            let buf = &bank[s.bank.min(bank.len() - 1)];
            let i = s.pos as usize;
            if i + 1 >= buf.len() {
                s.active = false;
                continue;
            }
            // Linear interpolation, so a re-pitched sample does not grind.
            let frac = s.pos - i as f32;
            out += (buf[i] + (buf[i + 1] - buf[i]) * frac) * s.gain;
            s.pos += s.rate;
        }
        out
    }

    fn level(&self) -> f32 {
        self.slots
            .iter()
            .filter(|s| s.active)
            .map(|s| s.gain)
            .fold(0.0f32, f32::max)
    }
}

struct NoiseCh {
    st: VoiceState,
    lfsr: u16,
    timer: f32,
    period: f32,
    short: bool,
}

impl NoiseCh {
    fn new() -> Self {
        NoiseCh {
            st: VoiceState::silent(),
            // The LFSR must never be seeded to zero: it would lock up.
            lfsr: 1,
            timer: 0.0,
            period: NOISE_PERIODS[8],
            short: false,
        }
    }

    fn tick_frame(&mut self) {
        let target = self.st.tick();
        self.st.set_gain_target(target);
        if self.st.active {
            let d = self.st.inst;
            self.period = NOISE_PERIODS[(d.noise_idx as usize).min(15)];
            self.short = d.noise_short;
        }
    }

    fn clock(&mut self) {
        let bit = if self.short { 6 } else { 1 };
        let feedback = (self.lfsr & 1) ^ ((self.lfsr >> bit) & 1);
        self.lfsr >>= 1;
        self.lfsr |= feedback << 14;
    }

    fn sample(&mut self) -> f32 {
        self.st.gain += self.st.gain_step;
        if self.st.gain <= 0.0001 && !self.st.active {
            self.st.gain = 0.0;
            return 0.0;
        }
        self.timer -= CYCLES_PER_SAMPLE;
        while self.timer <= 0.0 {
            self.timer += self.period;
            self.clock();
        }
        // The hardware silences the channel when bit 0 is set.
        if self.lfsr & 1 == 0 {
            self.st.gain.clamp(0.0, 15.0)
        } else {
            0.0
        }
    }
}

// ---------------------------------------------------------------------------
// Synth
// ---------------------------------------------------------------------------

/// Where the muffled zone filter parks its cutoff. Matches the recipe the
/// pre-rendered muffled SFX bank was baked with, so music and effects go
/// underwater together.
pub const ZONE_CUTOFF_HZ: f32 = 750.0;
const OPEN_CUTOFF_HZ: f32 = 18_000.0;

pub struct Synth {
    /// Lead, harmony and counter-melody.
    pulses: [PulseCh; 3],
    saw: SawCh,
    wave: WaveCh,
    tri: TriCh,
    /// Kick/snare and hats/shakers, so percussion stops masking itself.
    noise: [NoiseCh; 2],
    pcm: SampleCh,
    wavetables: [[f32; 32]; 4],
    samples: Vec<Vec<f32>>,
    frame_acc: u32,
    /// Absolute sample position — the one authoritative musical clock.
    pos: u64,
    hp: [OnePoleHp; 2],
    lp: [OnePoleLp; 2],
    zone_a: [OnePoleLp; 2],
    zone_b: [OnePoleLp; 2],
    zone_cut: f32,
    zone_target: f32,
    /// Stereo width, collapsed toward mono while the zone runs.
    width: f32,
    width_target: f32,
    gain: f32,
}

impl Default for Synth {
    fn default() -> Self {
        Self::new()
    }
}

impl Synth {
    pub fn new() -> Self {
        Synth {
            pulses: [PulseCh::new(), PulseCh::new(), PulseCh::new()],
            saw: SawCh::new(),
            wave: WaveCh::new(),
            tri: TriCh::new(),
            noise: [NoiseCh::new(), NoiseCh::new()],
            pcm: SampleCh::new(),
            wavetables: build_wavetables(),
            samples: build_samples(),
            frame_acc: 0,
            pos: 0,
            // Removes the mixer's DC offset (its output swings 0..1, not
            // -1..1) as well as sub-bass mud. The console's own 440 Hz
            // high-pass is deliberately *not* modelled: it makes the mix
            // thin, and this is background music, not console output.
            hp: [OnePoleHp::new(34.0), OnePoleHp::new(34.0)],
            lp: [OnePoleLp::new(14_000.0), OnePoleLp::new(14_000.0)],
            zone_a: [
                OnePoleLp::new(OPEN_CUTOFF_HZ),
                OnePoleLp::new(OPEN_CUTOFF_HZ),
            ],
            zone_b: [
                OnePoleLp::new(OPEN_CUTOFF_HZ),
                OnePoleLp::new(OPEN_CUTOFF_HZ),
            ],
            zone_cut: OPEN_CUTOFF_HZ,
            zone_target: OPEN_CUTOFF_HZ,
            width: 1.0,
            width_target: 1.0,
            // Equal-power panning drops a centred voice 3 dB relative to
            // a mono sum, so the master makes that back.
            gain: 2.3,
        }
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// Master trim, applied before the limiter.
    pub fn set_gain(&mut self, g: f32) {
        self.gain = g.clamp(0.0, 4.0);
    }

    /// Slide the whole mix underwater (or back) — the zone treatment.
    /// The stereo image narrows at the same time: closing in is part of
    /// what makes the moment read as concentration.
    pub fn set_muffled(&mut self, on: bool) {
        self.zone_target = if on { ZONE_CUTOFF_HZ } else { OPEN_CUTOFF_HZ };
        self.width_target = if on { 0.3 } else { 1.0 };
    }

    /// Release every sounding note. The gains ramp down over the current
    /// frame rather than snapping, so a hard cut between pieces does not
    /// click.
    pub fn all_notes_off(&mut self) {
        let mut states: Vec<&mut VoiceState> = Vec::with_capacity(VOICE_COUNT);
        states.extend(self.pulses.iter_mut().map(|c| &mut c.st));
        states.push(&mut self.saw.st);
        states.push(&mut self.wave.st);
        states.extend(self.noise.iter_mut().map(|c| &mut c.st));
        for st in states {
            st.active = false;
            st.frames_left = 0;
            st.set_gain_target(0.0);
        }
        self.tri.st.active = false;
        self.tri.st.frames_left = 0;
        self.tri.gate_target = 0.0;
        self.tri.kick_frames = 0;
        self.pcm.stop();
    }

    pub fn note_on(&mut self, ev: &NoteEvent) {
        match ev.voice() {
            Voice::Lead => self.pulses[0].st.start(ev),
            Voice::Harmony => self.pulses[1].st.start(ev),
            Voice::Counter => self.pulses[2].st.start(ev),
            Voice::Saw => self.saw.st.start(ev),
            Voice::Wave => self.wave.st.start(ev),
            Voice::Bass => self.tri.st.start(ev),
            Voice::Perc => {
                self.noise[0].st.start(ev);
                if ev.inst == Inst::Kick {
                    self.tri.kick();
                }
            }
            Voice::Hat => self.noise[1].st.start(ev),
            Voice::Sample => self.pcm.start(ev),
        }
    }

    fn tick_frame(&mut self) {
        // Exponential glide, ~4 frames to close. Fast enough to feel like
        // a filter slamming shut, slow enough not to click.
        self.zone_cut += (self.zone_target - self.zone_cut) * 0.22;
        self.width += (self.width_target - self.width) * 0.18;
        for i in 0..2 {
            self.zone_a[i].set_cutoff(self.zone_cut);
            self.zone_b[i].set_cutoff(self.zone_cut);
        }
        for p in &mut self.pulses {
            p.tick_frame();
        }
        self.saw.tick_frame();
        self.wave.tick_frame();
        self.tri.tick_frame();
        for n in &mut self.noise {
            n.tick_frame();
        }
    }

    /// One stereo frame. The two samples are emitted interleaved by the
    /// caller.
    pub fn next_frame(&mut self) -> (f32, f32) {
        if self.frame_acc == 0 {
            self.tick_frame();
        }
        self.frame_acc += 1;
        if self.frame_acc >= SAMPLES_PER_FRAME {
            self.frame_acc = 0;
        }
        self.pos += 1;

        let p1 = self.pulses[0].sample();
        let p2 = self.pulses[1].sample();
        let p3 = self.pulses[2].sample();
        let saw = self.saw.sample();
        let wav = self.wave.sample(&self.wavetables);
        let t = self.tri.sample();
        let n0 = self.noise[0].sample();
        let n1 = self.noise[1].sample();
        let pcm = self.pcm.sample(&self.samples);

        // The console's nonlinear mixer, on the four channels the console
        // actually had. This self-compression is why NES music never clips
        // and why stacked channels glue together; a linear mix is the usual
        // reason a from-scratch NES synth sounds harsh and too loud.
        //
        // It combines channels before we can pan them, so its output is
        // then split back across the contributing channels in proportion
        // to their own amplitudes — the compression stays correct while
        // each voice still gets its own place in the image.
        let p = p1 + p2;
        let pulse_out = if p > 0.0 {
            95.52 / (8128.0 / p + 100.0)
        } else {
            0.0
        };
        let tnd = 3.0 * t + 2.0 * n0;
        let tnd_out = if tnd > 0.0 {
            163.67 / (24329.0 / tnd + 100.0)
        } else {
            0.0
        };

        let w = self.width;
        let mut left = 0.0;
        let mut right = 0.0;
        let mut place = |v: Voice, amp: f32| {
            if amp == 0.0 {
                return;
            }
            let (l, r) = pan_gains(VOICE_PAN[v as usize], w);
            left += amp * l;
            right += amp * r;
        };
        if p > 0.0 {
            place(Voice::Lead, pulse_out * (p1 / p));
            place(Voice::Harmony, pulse_out * (p2 / p));
        }
        if tnd > 0.0 {
            place(Voice::Bass, tnd_out * (3.0 * t / tnd));
            place(Voice::Perc, tnd_out * (2.0 * n0 / tnd));
        }
        // Expansion audio mixed in linearly, exactly as a cartridge chip
        // would have. The scale factors put a full-scale expansion channel
        // at roughly the level of one console pulse.
        place(Voice::Counter, p3 / 15.0 * 0.105);
        place(Voice::Saw, saw / 31.0 * 0.125);
        place(Voice::Wave, wav / 15.0 * 0.100);
        place(Voice::Hat, n1 / 15.0 * 0.070);
        // The samples are already bipolar and normalized, so they take a
        // level rather than a scale factor.
        place(Voice::Sample, pcm * 0.16);

        let mut out = [left, right];
        for (i, s) in out.iter_mut().enumerate() {
            *s = self.hp[i].step(*s);
            *s = self.lp[i].step(*s);
            *s = self.zone_b[i].step(self.zone_a[i].step(*s));
            // tanh limiter: musically transparent until it isn't.
            *s = (*s * self.gain).tanh();
        }
        (out[0], out[1])
    }

    /// Fill an interleaved stereo buffer (L, R, L, R, ...).
    pub fn render(&mut self, out: &mut [f32]) {
        for frame in out.chunks_mut(2) {
            let (l, r) = self.next_frame();
            frame[0] = l;
            if frame.len() > 1 {
                frame[1] = r;
            }
        }
    }

    /// Peak amplitude currently sounding, 0..1-ish — the background
    /// visualizer's energy input.
    pub fn envelope(&self) -> f32 {
        let p: f32 = self.pulses.iter().map(|c| c.st.gain).sum::<f32>() / 45.0;
        let e = (self.saw.st.gain + self.wave.st.gain) / 30.0 * 0.6;
        let t = self.tri.gate * 0.35;
        let n = (self.noise[0].st.gain + self.noise[1].st.gain) / 30.0 * 0.5;
        (p + e + t + n + self.pcm.level() * 0.4).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(inst: Inst, midi: u8, frames: u16) -> NoteEvent {
        NoteEvent {
            at: 0,
            inst,
            midi,
            vel: 100,
            frames,
            arp: None,
            glide: 0,
        }
    }

    /// `frames` stereo frames, summed to mono — most of these tests care
    /// about content, not placement.
    fn mono(s: &mut Synth, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|_| {
                let (l, r) = s.next_frame();
                l + r
            })
            .collect()
    }

    /// Total absolute level per side over `frames`.
    fn sides(s: &mut Synth, frames: usize) -> (f32, f32) {
        let mut l_sum = 0.0;
        let mut r_sum = 0.0;
        for _ in 0..frames {
            let (l, r) = s.next_frame();
            l_sum += l.abs();
            r_sum += r.abs();
        }
        (l_sum, r_sum)
    }

    #[test]
    fn silence_is_actually_silent() {
        let mut s = Synth::new();
        let mut buf = vec![0.0; 4800];
        s.render(&mut buf);
        assert!(buf.iter().all(|v| v.abs() < 1e-4), "idle synth is not quiet");
    }

    #[test]
    fn a_note_makes_sound_and_then_stops() {
        let mut s = Synth::new();
        s.note_on(&ev(Inst::Pluck, 69, 10));
        let mut buf = vec![0.0; SAMPLE_RATE as usize / 2];
        s.render(&mut buf);
        let early: f32 = buf[..4000].iter().map(|v| v.abs()).sum();
        let late: f32 = buf[20000..24000].iter().map(|v| v.abs()).sum();
        assert!(early > 1.0, "note produced no sound ({early})");
        assert!(late < early * 0.1, "note never released ({early} -> {late})");
    }

    #[test]
    fn output_never_leaves_the_rails() {
        let mut s = Synth::new();
        // Every channel at once, loudest instruments, for a second.
        for _ in 0..12 {
            s.note_on(&ev(Inst::Sustain, 84, 60));
            s.note_on(&ev(Inst::Organ, 72, 60));
            s.note_on(&ev(Inst::Arp, 79, 60));
            s.note_on(&ev(Inst::SawLead, 60, 60));
            s.note_on(&ev(Inst::Bell, 67, 60));
            s.note_on(&ev(Inst::Bass, 36, 60));
            s.note_on(&ev(Inst::Kick, 0, 8));
            s.note_on(&ev(Inst::Hat, 0, 6));
            let mut buf = vec![0.0; 8000];
            s.render(&mut buf);
            assert!(
                buf.iter().all(|v| v.is_finite() && v.abs() <= 1.0),
                "output left [-1, 1]"
            );
        }
    }

    #[test]
    fn every_voice_can_be_heard() {
        // A note on each channel must actually reach the mix — a missing
        // arm in note_on() or the mixer would be silent, not a crash.
        for inst in crate::ALL_INSTRUMENTS {
            let mut s = Synth::new();
            let midi = if inst == Inst::Bass { 40 } else { 67 };
            s.note_on(&ev(inst, midi, 20));
            let level: f32 = mono(&mut s, 16_000).iter().map(|v| v.abs()).sum();
            assert!(level > 0.5, "{inst:?} produced nothing ({level})");
        }
    }

    #[test]
    fn voices_are_spread_across_the_stereo_image() {
        let side_bias = |inst: Inst, midi: u8| {
            let mut s = Synth::new();
            s.note_on(&ev(inst, midi, 40));
            let (l, r) = sides(&mut s, 20_000);
            (r - l) / (r + l).max(1e-6)
        };
        // The harmony sits left, the counter-melody right — an echo has to
        // land opposite the lead or it reads as a doubling, not as space.
        assert!(side_bias(Inst::Organ, 67) < -0.15, "harmony is not left");
        assert!(side_bias(Inst::Echo, 67) > 0.15, "counter is not right");
        assert!(side_bias(Inst::Hat, 0) > 0.10, "hats are not right");
        // The low end must stay centred or the mix falls apart.
        assert!(side_bias(Inst::Bass, 40).abs() < 0.02, "bass is off centre");
    }

    #[test]
    fn the_zone_narrows_the_image() {
        let spread = |muffled: bool| {
            let mut s = Synth::new();
            s.set_muffled(muffled);
            // Let the width glide settle.
            s.note_on(&ev(Inst::Organ, 67, 200));
            mono(&mut s, 20_000);
            s.note_on(&ev(Inst::Organ, 67, 200));
            let (l, r) = sides(&mut s, 20_000);
            (l - r).abs() / (l + r).max(1e-6)
        };
        let open = spread(false);
        let closed = spread(true);
        assert!(
            closed < open * 0.6,
            "zone did not pull the image in: {open} -> {closed}"
        );
    }

    #[test]
    fn the_two_noise_channels_do_not_mask_each_other() {
        // The whole reason for a second noise channel: a hat landing on
        // the same step as a kick used to be swallowed whole.
        // Well below the limiter's knee, so this measures the mix and not
        // tanh's incremental gain.
        let render = |kick: bool, hat: bool| {
            let mut s = Synth::new();
            s.set_gain(0.25);
            if kick {
                s.note_on(&ev(Inst::Kick, 0, 8));
            }
            if hat {
                s.note_on(&ev(Inst::Hat, 0, 6));
            }
            // Energy, not mean absolute level: the two sources are
            // uncorrelated noise, and only their energies add.
            mono(&mut s, 8_000).iter().map(|v| v * v).sum::<f32>()
        };
        let level = |with_hat: bool| render(true, with_hat);
        let hat_alone = render(false, true);
        let kick_only = level(false);
        let both = level(true);
        // The hat's own contribution has to survive the kick essentially
        // intact. On one shared channel it did not survive at all: the
        // later note simply took the channel.
        let survived = both - kick_only;
        assert!(
            survived > hat_alone * 0.5,
            "the hat lost {:.0}% under the kick (alone {hat_alone:.1}, survived {survived:.1})",
            100.0 * (1.0 - survived / hat_alone)
        );
    }

    #[test]
    fn pitch_quantization_stays_close_to_equal_temperament() {
        // The pulse channels bottom out around A1 on real hardware (the
        // period register runs out), which is why the bass lives on the
        // triangle — it counts at half the rate and reaches an octave
        // lower.
        for midi in 33..=100 {
            let want = midi_to_hz(midi as f32);
            let got = quantize_pulse(want);
            let cents = 1200.0 * (got / want).log2();
            // The period register runs out of resolution up high, so the
            // top octaves genuinely detune — that is the hardware's own
            // error and a real part of the sound. It just must never grow
            // large enough to read as a wrong note.
            let limit = if midi < 84 { 8.0 } else { 30.0 };
            assert!(cents.abs() < limit, "pulse midi {midi}: {cents} cents off");
        }
        for midi in 21..=80 {
            let want = midi_to_hz(midi as f32);
            let cents = 1200.0 * (quantize_triangle(want) / want).log2();
            assert!(cents.abs() < 12.0, "triangle midi {midi}: {cents} cents off");
        }
    }

    #[test]
    fn noise_lfsr_never_locks_up() {
        let mut n = NoiseCh::new();
        let mut ones = 0;
        for _ in 0..10_000 {
            n.clock();
            assert_ne!(n.lfsr, 0, "LFSR hit the absorbing zero state");
            ones += (n.lfsr & 1) as u32;
        }
        // A 15-bit maximal LFSR is very close to balanced.
        assert!((3_000..7_000).contains(&ones), "noise is not noisy: {ones}");
    }

    #[test]
    fn polyblep_suppresses_aliasing() {
        // Render a high 12.5%-duty note and measure how much energy lands
        // between the harmonics. This is the regression guard for the
        // "sour chiptune" failure mode.
        let midi = 100u8; // ~2637 Hz
        let f0 = quantize_pulse(midi_to_hz(midi as f32));
        let mut s = Synth::new();
        s.note_on(&NoteEvent {
            at: 0,
            inst: Inst::Stab,
            midi,
            vel: 127,
            frames: 600,
            arp: None,
            glide: 0,
        });
        let n = 16384;
        let buf = mono(&mut s, n);

        // Goertzel-style DFT probe at a set of frequencies.
        let power = |hz: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &v) in buf.iter().enumerate() {
                let w = std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE as f32;
                // Hann window keeps harmonic leakage from masquerading as
                // an alias.
                let win = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
                re += v * win * w.cos();
                im += v * win * w.sin();
            }
            (re * re + im * im).sqrt()
        };

        let fundamental = power(f0);
        assert!(fundamental > 1.0, "no fundamental at all");
        // Sweep the band below the fundamental, skipping nothing: with
        // naive squares this is littered with images at -13 to -19 dBc.
        let mut worst: f32 = 0.0;
        let mut hz = 120.0;
        while hz < f0 - 200.0 {
            worst = worst.max(power(hz) / fundamental);
            hz += 40.0;
        }
        let dbc = 20.0 * worst.log10();
        assert!(dbc < -34.0, "sub-fundamental aliasing at {dbc:.1} dBc");
    }

    #[test]
    fn muffling_removes_treble() {
        let bright = |muffled: bool| {
            let mut s = Synth::new();
            s.set_muffled(muffled);
            // Let the filter settle before measuring.
            s.note_on(&ev(Inst::Stab, 96, 200));
            mono(&mut s, 12_000);
            s.note_on(&ev(Inst::Stab, 96, 200));
            let buf = mono(&mut s, 16_000);
            // Crude treble measure: mean absolute first difference.
            buf.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / buf.len() as f32
        };
        let dry = bright(false);
        let wet = bright(true);
        assert!(wet < dry * 0.5, "zone filter barely did anything: {dry} -> {wet}");
    }
}
