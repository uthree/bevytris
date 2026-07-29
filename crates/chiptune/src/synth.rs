//! The synthesizer: two pulse channels, a triangle and a noise channel,
//! mixed through the NES's own nonlinear mixer and filter chain.
//!
//! Two deliberate choices are worth knowing about before touching this:
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

use crate::{
    FRAME_RATE, Inst, InstDef, NoteEvent, SAMPLES_PER_FRAME, SAMPLE_RATE, Voice, inst_def,
};

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
            gain: 0.0,
            gain_step: 0.0,
        }
    }

    fn start(&mut self, ev: &NoteEvent) {
        self.inst = inst_def(ev.inst);
        self.which = ev.inst;
        self.active = true;
        self.cursor = 0;
        self.frames_left = ev.frames.max(1);
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
        let d = self.inst;
        if d.vib_depth > 0.0 && self.frames_played > d.vib_delay {
            let t = (self.frames_played - d.vib_delay) as f32 / FRAME_RATE as f32;
            m += d.vib_depth * (std::f32::consts::TAU * d.vib_rate * t).sin();
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
    p1: PulseCh,
    p2: PulseCh,
    tri: TriCh,
    noi: NoiseCh,
    frame_acc: u32,
    /// Absolute sample position — the one authoritative musical clock.
    pos: u64,
    hp: OnePoleHp,
    lp: OnePoleLp,
    zone_a: OnePoleLp,
    zone_b: OnePoleLp,
    zone_cut: f32,
    zone_target: f32,
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
            p1: PulseCh::new(),
            p2: PulseCh::new(),
            tri: TriCh::new(),
            noi: NoiseCh::new(),
            frame_acc: 0,
            pos: 0,
            // Removes the mixer's DC offset (its output swings 0..1, not
            // -1..1) as well as sub-bass mud. The console's own 440 Hz
            // high-pass is deliberately *not* modelled: it makes the mix
            // thin, and this is background music, not console output.
            hp: OnePoleHp::new(34.0),
            lp: OnePoleLp::new(14_000.0),
            zone_a: OnePoleLp::new(OPEN_CUTOFF_HZ),
            zone_b: OnePoleLp::new(OPEN_CUTOFF_HZ),
            zone_cut: OPEN_CUTOFF_HZ,
            zone_target: OPEN_CUTOFF_HZ,
            gain: 1.7,
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
    pub fn set_muffled(&mut self, on: bool) {
        self.zone_target = if on { ZONE_CUTOFF_HZ } else { OPEN_CUTOFF_HZ };
    }

    pub fn note_on(&mut self, ev: &NoteEvent) {
        match ev.voice() {
            Voice::Lead => self.p1.st.start(ev),
            Voice::Harmony => self.p2.st.start(ev),
            Voice::Bass => self.tri.st.start(ev),
            Voice::Perc => {
                self.noi.st.start(ev);
                if ev.inst == Inst::Kick {
                    self.tri.kick();
                }
            }
        }
    }

    fn tick_frame(&mut self) {
        // Exponential glide, ~4 frames to close. Fast enough to feel like
        // a filter slamming shut, slow enough not to click.
        self.zone_cut += (self.zone_target - self.zone_cut) * 0.22;
        self.zone_a.set_cutoff(self.zone_cut);
        self.zone_b.set_cutoff(self.zone_cut);
        self.p1.tick_frame();
        self.p2.tick_frame();
        self.tri.tick_frame();
        self.noi.tick_frame();
    }

    pub fn next_sample(&mut self) -> f32 {
        if self.frame_acc == 0 {
            self.tick_frame();
        }
        self.frame_acc += 1;
        if self.frame_acc >= SAMPLES_PER_FRAME {
            self.frame_acc = 0;
        }
        self.pos += 1;

        let p = self.p1.sample() + self.p2.sample();
        let t = self.tri.sample();
        let n = self.noi.sample();

        // The console's nonlinear mixer. This self-compression is why NES
        // music never clips and why stacked channels glue together; a
        // linear mix is the usual reason a from-scratch NES synth sounds
        // harsh and too loud.
        let pulse_out = if p > 0.0 {
            95.52 / (8128.0 / p + 100.0)
        } else {
            0.0
        };
        let tnd = 3.0 * t + 2.0 * n;
        let tnd_out = if tnd > 0.0 {
            163.67 / (24329.0 / tnd + 100.0)
        } else {
            0.0
        };

        let mut s = self.hp.step(pulse_out + tnd_out);
        s = self.lp.step(s);
        s = self.zone_b.step(self.zone_a.step(s));
        // tanh limiter: musically transparent until it isn't.
        (s * self.gain).tanh()
    }

    pub fn render(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = self.next_sample();
        }
    }

    /// Peak amplitude currently sounding, 0..1-ish — the background
    /// visualizer's energy input.
    pub fn envelope(&self) -> f32 {
        let p = (self.p1.st.gain + self.p2.st.gain) / 30.0;
        let t = self.tri.gate * 0.35;
        let n = self.noi.st.gain / 15.0 * 0.5;
        (p + t + n).min(1.0)
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
        }
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
        // Everything at once, loudest instruments, for a second.
        for _ in 0..12 {
            s.note_on(&ev(Inst::Sustain, 84, 60));
            s.note_on(&ev(Inst::Organ, 72, 60));
            s.note_on(&ev(Inst::Bass, 36, 60));
            s.note_on(&ev(Inst::Kick, 0, 8));
            let mut buf = vec![0.0; 4000];
            s.render(&mut buf);
            assert!(
                buf.iter().all(|v| v.is_finite() && v.abs() <= 1.0),
                "output left [-1, 1]"
            );
        }
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
        });
        let n = 16384;
        let mut buf = vec![0.0; n];
        s.render(&mut buf);

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
            let mut warm = vec![0.0; 12_000];
            s.note_on(&ev(Inst::Stab, 96, 200));
            s.render(&mut warm);
            let mut buf = vec![0.0; 16_000];
            s.note_on(&ev(Inst::Stab, 96, 200));
            s.render(&mut buf);
            // Crude treble measure: mean absolute first difference.
            buf.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / buf.len() as f32
        };
        let dry = bright(false);
        let wet = bright(true);
        assert!(wet < dry * 0.5, "zone filter barely did anything: {dry} -> {wet}");
    }
}
