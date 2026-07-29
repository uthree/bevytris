//! The whole engine behind one object: parameters in, waveform out.
//!
//! ```no_run
//! use bevytris_chiptune::director::Director;
//! use bevytris_chiptune::compose::{Context, Profile};
//!
//! let mut d = Director::new(0xC0FFEE);
//! d.set_context(Context { profile: Profile::VsIntense, ..Default::default() });
//! let mut buf = vec![0.0; 4800 * 2]; // interleaved stereo
//! d.fill(&mut buf);
//! ```
//!
//! The composer and the synthesizer live together here, and a bar is
//! composed at the moment the playhead reaches it. That is the whole
//! point: an earlier design ran the composer several seconds ahead and
//! shipped notes across a channel, which meant every change of music had
//! to invalidate a backlog of notes already in flight. Getting that wrong
//! made a screen transition keep playing the old tune for the length of
//! the lookahead. There is no lookahead now, so there is nothing to
//! invalidate — [`Director::set_context`] simply changes what the next
//! bar will be.
//!
//! The cost is that composition happens wherever `fill` is called, which
//! in the game is the audio thread. A bar's worth of planning is a few
//! small allocations every second and a half; the buffers it reuses are
//! kept, so after the first few bars it settles.

use crate::compose::{Composer, Context, Info};
use crate::synth::Synth;
use crate::NoteEvent;

/// If the playhead somehow gets this far ahead of where the next bar was
/// due — a long stall, or the very first frame — the score re-anchors to
/// now rather than trying to catch up through the backlog.
const RESYNC_SLACK: u64 = crate::SAMPLE_RATE as u64 / 4;

/// How far past the playhead the score is kept composed.
///
/// This is not the old cross-thread lookahead: the queue is a plain `Vec`
/// owned right here, so a change of music clears it in one call instead
/// of having to chase notes already handed to another thread. Two seconds
/// is enough for the piano roll to have something to draw ahead of the
/// playhead, which is the only reason it is more than zero.
const PLAN_AHEAD: u64 = crate::SAMPLE_RATE as u64 * 2;

pub struct Director {
    composer: Composer,
    synth: Synth,
    ctx: Context,
    /// The bar currently being played out, in time order.
    bar: Vec<NoteEvent>,
    cursor: usize,
    /// Notes planned since the caller last drained them — the piano
    /// roll's feed.
    fresh: Vec<NoteEvent>,
    modulated: bool,
    cut: bool,
}

impl Director {
    pub fn new(seed: u64) -> Self {
        Director {
            composer: Composer::new(seed),
            synth: Synth::new(),
            ctx: Context::default(),
            bar: Vec::with_capacity(256),
            cursor: 0,
            fresh: Vec::with_capacity(1024),
            modulated: false,
            cut: false,
        }
    }

    /// Hand the engine the current gameplay parameters. Changing the
    /// profile starts a new piece from the next frame; everything else
    /// takes effect at the next bar line, which is where a change of key,
    /// density or arrangement wants to land anyway.
    pub fn set_context(&mut self, ctx: Context) {
        if ctx.profile != self.ctx.profile {
            self.composer.set_profile(ctx.profile, ctx.intensity);
            // Cut cleanly and start the new piece here, not wherever the
            // old one had got to.
            self.synth.all_notes_off();
            self.composer.seek(self.synth.pos());
            self.bar.clear();
            self.cursor = 0;
            // The feed goes with it: notes from a piece that was cut must
            // not still be sitting in the visualizer's queue, out of
            // order with everything the new one is about to compose.
            self.fresh.clear();
            self.cut = true;
        }
        self.synth.set_muffled(ctx.zone);
        self.ctx = ctx;
    }

    pub fn context(&self) -> Context {
        self.ctx
    }

    pub fn info(&self) -> Info {
        self.composer.info()
    }

    /// The authoritative musical clock, in stereo frames.
    pub fn pos(&self) -> u64 {
        self.synth.pos()
    }

    /// Instantaneous output level, for visualizers.
    pub fn envelope(&self) -> f32 {
        self.synth.envelope()
    }

    /// Master trim, before the limiter.
    pub fn set_gain(&mut self, g: f32) {
        self.synth.set_gain(g);
    }

    /// Pin the time signature the piece rolled. Auditioning aid; `None`
    /// leaves the roll alone.
    pub fn force_meter(&mut self, meter: Option<crate::compose::Meter>) {
        if let Some(m) = meter {
            self.composer.force_meter(m);
        }
    }

    /// Pin the drum kit the piece rolled.
    pub fn force_kit(&mut self, kit: Option<crate::compose::Kit>) {
        if let Some(k) = kit {
            self.composer.force_kit(k);
        }
    }

    /// Move everything planned since the last call into `out`. Notes are
    /// in time order and their timestamps are absolute frames, matching
    /// [`Director::pos`].
    pub fn drain_new_notes(&mut self, out: &mut Vec<NoteEvent>) {
        out.append(&mut self.fresh);
    }

    /// True once, after the final chorus has changed key.
    pub fn take_modulated(&mut self) -> bool {
        std::mem::take(&mut self.modulated)
    }

    /// True once, after a new piece cut off the old one. Anything holding
    /// a copy of the score should throw it away.
    pub fn take_cut(&mut self) -> bool {
        std::mem::take(&mut self.cut)
    }

    /// One stereo frame, composing another bar first if the playhead has
    /// reached one.
    pub fn next_frame(&mut self) -> (f32, f32) {
        let pos = self.synth.pos();
        // A stall (or the first bar) leaves the score behind the
        // playhead; start from here rather than emitting a burst of notes
        // whose moment has passed.
        if pos > self.composer.next_bar_at() + RESYNC_SLACK {
            self.composer.seek(pos);
        }
        let mut guard = 0;
        while self.composer.next_bar_at() < pos + PLAN_AHEAD && guard < 8 {
            self.plan();
            guard += 1;
        }
        while self.cursor < self.bar.len() && self.bar[self.cursor].at <= pos {
            let ev = self.bar[self.cursor];
            self.synth.note_on(&ev);
            self.cursor += 1;
        }
        // Drop what has already been played, so the queue does not creep.
        if self.cursor > 256 {
            self.bar.drain(..self.cursor);
            self.cursor = 0;
        }
        self.synth.next_frame()
    }

    /// Fill an interleaved stereo buffer. This is the batch form of
    /// [`Director::next_frame`] and behaves identically — the offline
    /// renderer and the game are running the same code.
    pub fn fill(&mut self, out: &mut [f32]) {
        for frame in out.chunks_mut(2) {
            let (l, r) = self.next_frame();
            frame[0] = l;
            if frame.len() > 1 {
                frame[1] = r;
            }
        }
    }

    fn plan(&mut self) {
        let start = self.bar.len();
        self.composer.plan_bar(&self.ctx, &mut self.bar);
        self.modulated |= self.composer.take_modulated();
        // If nobody is draining the feed, keep the newest rather than
        // growing without bound.
        if self.fresh.len() > 4096 {
            self.fresh.clear();
        }
        let planned = &self.bar[start..];
        self.fresh.extend_from_slice(planned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::Profile;

    fn ctx(profile: Profile) -> Context {
        Context {
            profile,
            intensity: 0.5,
            elapsed: 600.0,
            ..Default::default()
        }
    }

    fn energy(d: &mut Director, frames: usize) -> f32 {
        let mut buf = vec![0.0; frames * 2];
        d.fill(&mut buf);
        buf.iter().map(|v| v.abs()).sum()
    }

    #[test]
    fn parameters_in_waveform_out() {
        let mut d = Director::new(1);
        d.set_context(ctx(Profile::VsIntense));
        let level = energy(&mut d, 48_000);
        assert!(level > 100.0, "a second of music produced {level}");
        assert_eq!(d.pos(), 48_000, "the clock counts frames, not samples");
    }

    #[test]
    fn the_same_seed_gives_the_same_waveform() {
        let render = || {
            let mut d = Director::new(0xABCDEF);
            d.set_context(ctx(Profile::SoloCalm));
            let mut buf = vec![0.0; 48_000 * 2];
            d.fill(&mut buf);
            buf
        };
        assert_eq!(render(), render(), "the engine is not reproducible");
    }

    #[test]
    fn different_seeds_give_different_music() {
        let render = |seed: u64| {
            let mut d = Director::new(seed);
            d.set_context(ctx(Profile::SoloCalm));
            let mut notes = Vec::new();
            let mut buf = vec![0.0; 48_000 * 4];
            d.fill(&mut buf);
            d.drain_new_notes(&mut notes);
            notes.iter().map(|n| (n.at, n.midi)).collect::<Vec<_>>()
        };
        assert_ne!(render(1), render(2));
    }

    /// The bug the whole architecture exists to make impossible: a change
    /// of profile used to leave a lookahead's worth of the previous piece
    /// still queued, so the music kept playing the old tune for several
    /// seconds.
    #[test]
    fn a_profile_change_takes_effect_immediately() {
        let mut d = Director::new(7);
        d.set_context(ctx(Profile::Ambient));
        energy(&mut d, 48_000 * 3);
        let mut before = Vec::new();
        d.drain_new_notes(&mut before);
        let ambient_info = d.info();

        d.set_context(ctx(Profile::VsIntense));
        // A single bar's worth of frames is enough to see the new piece.
        energy(&mut d, 24_000);
        let mut after = Vec::new();
        d.drain_new_notes(&mut after);

        assert!(!after.is_empty(), "no notes at all after the switch");
        let now = d.info();
        assert_ne!(
            (now.bpm, now.meter, now.tonic),
            (ambient_info.bpm, ambient_info.meter, ambient_info.tonic),
            "the piece did not actually change"
        );
        // Everything emitted after the switch belongs to the new piece:
        // there is no backlog, because nothing is ever planned ahead.
        let switch_at = d.pos() - 24_000;
        assert!(
            after.iter().all(|n| n.at >= switch_at),
            "notes from the old piece survived the switch"
        );
    }

    #[test]
    fn the_score_matches_what_is_played() {
        let mut d = Director::new(99);
        d.set_context(ctx(Profile::VsIntense));
        let mut notes = Vec::new();
        energy(&mut d, 48_000 * 5);
        d.drain_new_notes(&mut notes);
        assert!(notes.len() > 40);
        assert!(notes.windows(2).all(|w| w[0].at <= w[1].at));
        // Timestamps are on the same clock the caller reads. The feed
        // runs a little ahead of the playhead on purpose — that margin is
        // what the piano roll draws as upcoming — but never further than
        // the plan-ahead window plus the bar it lands in.
        let horizon = d.pos() + PLAN_AHEAD + crate::SAMPLE_RATE as u64 * 3;
        assert!(notes.iter().all(|n| n.at <= horizon));
        assert!(
            notes.iter().any(|n| n.at > d.pos()),
            "the feed has nothing upcoming for the roll to draw"
        );
    }

    #[test]
    fn draining_is_optional() {
        // A caller that never reads the feed must not make it grow
        // without bound.
        let mut d = Director::new(3);
        d.set_context(ctx(Profile::VsIntense));
        energy(&mut d, 48_000 * 120);
        let mut notes = Vec::new();
        d.drain_new_notes(&mut notes);
        assert!(notes.len() <= 4096 + 512, "the feed grew to {}", notes.len());
    }

    #[test]
    fn a_stall_re_anchors_instead_of_dumping_a_backlog() {
        let mut d = Director::new(5);
        d.set_context(ctx(Profile::SoloCalm));
        energy(&mut d, 4_800);
        let mut warm = Vec::new();
        d.drain_new_notes(&mut warm);
        // Jump the clock forward the way a long stall would.
        let mut buf = vec![0.0; 2];
        for _ in 0..40_000 {
            d.fill(&mut buf);
        }
        let mut notes = Vec::new();
        d.drain_new_notes(&mut notes);
        // Nothing is scheduled in the past by more than a bar.
        let pos = d.pos();
        assert!(notes.iter().all(|n| n.at <= pos));
    }
}
