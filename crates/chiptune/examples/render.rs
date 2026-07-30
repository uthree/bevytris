//! Offline renderer — the development loop for the music engine.
//!
//! ```text
//! cargo run -p bevytris-chiptune --release --example render -- \
//!     --profile vs --seed 42 --secs 90 --ramp --out vs.wav
//! ```
//!
//! `--smooth 0..1` (or `auto`) overrides how stepwise the melodies are:
//! 0 is the widest intervals this writes, 1 is very nearly stepwise.
//!
//! `--ramp` sweeps the intensity from 0 to 1 across the render, which is
//! how the tempo ladder and the layer schedule get auditioned without
//! playing the game badly on purpose.

use bevytris_chiptune::compose::{ChordStyle, Context, Kit, Meter, Profile};
use bevytris_chiptune::director::Director;
use bevytris_chiptune::{SAMPLE_RATE, wav};

fn main() {
    let mut seed: u64 = 42;
    let mut secs: f32 = 60.0;
    let mut profile = Profile::SoloCalm;
    let mut out_path = String::from("music.wav");
    let mut ramp = false;
    let mut zone_at: Option<f32> = None;
    let mut intensity = 0.35f32;
    let mut meter: Option<Meter> = None;
    let mut kit: Option<Kit> = None;
    let mut smooth: Option<f32> = None;
    let mut lead: Option<bevytris_chiptune::Inst> = None;
    let mut chords: Option<ChordStyle> = None;

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
                let raw = next(i);
                seed = bevytris_chiptune::parse_seed(&raw)
                    .unwrap_or_else(|| panic!("seed {raw} is not a number"));
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
                    "zen" => Profile::Zen,
                    "zenpeak" | "peak" => Profile::ZenPeak,
                    other => panic!("unknown profile {other}"),
                };
                i += 2;
            }
            "--meter" => {
                meter = Some(match next(i).as_str() {
                    "4" | "4/4" => Meter::Four,
                    "3" | "3/4" => Meter::Three,
                    "6" | "6/8" => Meter::Six,
                    other => panic!("unknown meter {other} (try 4, 3 or 6)"),
                });
                i += 2;
            }
            "--kit" => {
                kit = Some(match next(i).as_str() {
                    "chip" | "nes" => Kit::Chip,
                    "pcm" | "sampled" => Kit::Sampled,
                    other => panic!("unknown kit {other} (try chip or pcm)"),
                });
                i += 2;
            }
            "--out" => {
                out_path = next(i);
                i += 2;
            }
            "--smooth" => {
                let raw = next(i);
                smooth = if raw == "auto" {
                    None
                } else {
                    Some(raw.parse().expect("smooth must be a number 0..1 or auto"))
                };
                i += 2;
            }
            "--lead" => {
                use bevytris_chiptune::Inst;
                lead = match next(i).as_str() {
                    "auto" => None,
                    "pluck" => Some(Inst::Pluck),
                    "sustain" => Some(Inst::Sustain),
                    "soft" => Some(Inst::Soft),
                    "piano" => Some(Inst::Piano),
                    "guitar" => Some(Inst::Guitar),
                    "marimba" => Some(Inst::Marimba),
                    "brass" => Some(Inst::Brass),
                    other => panic!("unknown lead {other}"),
                };
                i += 2;
            }
            "--chords" => {
                chords = match next(i).as_str() {
                    "auto" => None,
                    "strum" => Some(ChordStyle::Strum),
                    "held" | "sustain" => Some(ChordStyle::Sustain),
                    other => panic!("unknown chord style {other} (try strum or held)"),
                };
                i += 2;
            }
            "--ramp" => {
                ramp = true;
                i += 1;
            }
            other => panic!("unknown flag {other}"),
        }
    }

    // Stereo frames; the buffer below is interleaved.
    let total = (secs * SAMPLE_RATE as f32) as usize;
    let mut director = Director::new(seed);
    director.set_context(Context {
        profile,
        intensity: if ramp { 0.0 } else { intensity },
        elapsed: 0.0,
        ..Default::default()
    });
    director.pin_meter(meter);
    director.pin_kit(kit);
    director.pin_smoothness(smooth);
    director.pin_lead(lead);
    director.pin_chord_style(chords);

    let mut samples = vec![0.0f32; total * 2];
    let mut notes = Vec::new();

    // Chunked only so the context can move; the engine is the same one
    // the game runs, called the same way.
    let chunk = SAMPLE_RATE as usize / 30;
    let mut pos = 0usize;
    while pos < total {
        let t = pos as f32 / SAMPLE_RATE as f32;
        director.set_context(Context {
            profile,
            intensity: if ramp {
                (t / secs).clamp(0.0, 1.0)
            } else {
                intensity
            },
            transpose: 0,
            zone: zone_at.is_some_and(|z| t >= z && t < z + 8.0),
            elapsed: t,
        });
        let end = (pos + chunk).min(total);
        director.fill(&mut samples[pos * 2..end * 2]);
        director.drain_new_notes(&mut notes);
        pos = end;
    }

    let peak = samples.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let info = director.info();
    // How far the mix leans left or right overall — a sanity check that
    // the panning is doing something without being lopsided.
    let (mut l, mut r) = (0.0f32, 0.0f32);
    for f in samples.chunks(2) {
        l += f[0].abs();
        r += f[1].abs();
    }
    println!(
        "{} · {} kit | {} notes | peak {:.3} ({:.1} dBFS) | rms {:.3} ({:.1} dBFS) | balance {:+.1}%",
        info.label(),
        info.kit.name(),
        notes.len(),
        peak,
        20.0 * peak.max(1e-9).log10(),
        rms,
        20.0 * rms.max(1e-9).log10(),
        100.0 * (r - l) / (r + l).max(1e-9),
    );

    std::fs::write(&out_path, wav::encode(&samples, 2)).expect("failed to write WAV");
    println!("wrote {out_path} ({secs} s)");
}
