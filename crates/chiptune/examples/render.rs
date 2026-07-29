//! Offline renderer — the development loop for the music engine.
//!
//! ```text
//! cargo run -p bevytris-chiptune --release --example render -- \
//!     --profile vs --seed 42 --secs 90 --ramp --out vs.wav
//! ```
//!
//! `--ramp` sweeps the intensity from 0 to 1 across the render, which is
//! how the tempo ladder and the layer schedule get auditioned without
//! playing the game badly on purpose.

use bevytris_chiptune::compose::{Composer, Context, Profile};
use bevytris_chiptune::synth::Synth;
use bevytris_chiptune::{NoteEvent, SAMPLE_RATE, wav};

fn main() {
    let mut seed: u64 = 42;
    let mut secs: f32 = 60.0;
    let mut profile = Profile::SoloCalm;
    let mut out_path = String::from("music.wav");
    let mut ramp = false;
    let mut zone_at: Option<f32> = None;
    let mut intensity = 0.35f32;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let next = |i: usize| -> String {
            args.get(i + 1)
                .unwrap_or_else(|| panic!("{} needs a value", args[i]))
                .clone()
        };
        match args[i].as_str() {
            "--seed" => {
                seed = next(i).parse().expect("seed must be a number");
                i += 2;
            }
            "--secs" => {
                secs = next(i).parse().expect("secs must be a number");
                i += 2;
            }
            "--intensity" => {
                intensity = next(i).parse().expect("intensity must be a number");
                i += 2;
            }
            "--zone-at" => {
                zone_at = Some(next(i).parse().expect("zone-at must be a number"));
                i += 2;
            }
            "--profile" => {
                profile = match next(i).as_str() {
                    "ambient" | "menu" | "title" => Profile::Ambient,
                    "solo" | "calm" => Profile::SoloCalm,
                    "vs" | "versus" | "intense" => Profile::VsIntense,
                    "victory" | "win" => Profile::Victory,
                    other => panic!("unknown profile {other}"),
                };
                i += 2;
            }
            "--out" => {
                out_path = next(i);
                i += 2;
            }
            "--ramp" => {
                ramp = true;
                i += 1;
            }
            other => panic!("unknown flag {other}"),
        }
    }

    let total = (secs * SAMPLE_RATE as f32) as usize;
    let mut synth = Synth::new();
    let mut composer = Composer::new(seed);
    composer.set_profile(profile, if ramp { 0.0 } else { intensity });

    let mut pending: Vec<NoteEvent> = Vec::new();
    let mut cursor = 0usize;
    let mut samples = vec![0.0f32; total];
    let lookahead = SAMPLE_RATE as u64 * 4;

    // Chunked so the composer sees a moving playhead, exactly as it does
    // in game.
    let chunk = SAMPLE_RATE as usize / 30;
    let mut pos = 0usize;
    while pos < total {
        let t = pos as f32 / SAMPLE_RATE as f32;
        let ctx = Context {
            profile,
            intensity: if ramp {
                (t / secs).clamp(0.0, 1.0)
            } else {
                intensity
            },
            transpose: 0,
            zone: zone_at.is_some_and(|z| t >= z && t < z + 8.0),
            elapsed: t,
        };
        synth.set_muffled(ctx.zone);
        composer.advance(pos as u64, lookahead, &ctx, &mut pending);

        let end = (pos + chunk).min(total);
        for s in samples.iter_mut().take(end).skip(pos) {
            let now = synth.pos();
            while cursor < pending.len() && pending[cursor].at <= now {
                synth.note_on(&pending[cursor]);
                cursor += 1;
            }
            *s = synth.next_sample();
        }
        pos = end;
    }

    let peak = samples.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let info = composer.info();
    println!(
        "{} | {} notes | peak {:.3} ({:.1} dBFS) | rms {:.3} ({:.1} dBFS)",
        info.label(),
        pending.len(),
        peak,
        20.0 * peak.max(1e-9).log10(),
        rms,
        20.0 * rms.max(1e-9).log10(),
    );

    std::fs::write(&out_path, wav::encode(&samples)).expect("failed to write WAV");
    println!("wrote {out_path} ({secs} s)");
}
