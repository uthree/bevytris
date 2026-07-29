# bevytris

A guideline-flavored Tetris clone written in Rust with [Bevy Engine](https://bevy.org) 0.19.

![title screen](docs/screenshot.png)

## Features

- **Solo modes** — Marathon (classic endless, guideline gravity curve),
  Sprint (40-line race) and Dig (cheese race), with persistent personal bests
- **VS CPU mode** — a 30-stage ladder of computer opponents with distinct
  personalities (Balanced / Rusher / Thinker / Spinner), human-like blunder
  rates that fade out as stages climb, garbage attack & cancellation rules
  modeled after modern versus games, a **time-based gravity ramp** (both
  boards speed up one level every 25 s — cleared lines never change the
  pace), **margin time** (long rounds scale everyone's attack up),
  **first-to-2 rounds** (boss stages 10/20/30 are first-to-3), and an
  **S/A/B/C/D grade** on every stage clear based on dominance,
  attack-per-minute and style. Progress and best grades persist.
- **ZONE BATTLE** — a second 30-stage campaign with a Tetris-Effect-style
  super move: cancelling or digging garbage charges a gauge; firing it stops
  time, banks every cleared row at the bottom of the field and launches them
  all as one attack when the zone ends.
- **CUSTOM MATCH** — build your own versus rules: CPU level (1-30) and
  playstyle, first-to-N, zone gauges on/off, margin time, fixed or ramping
  gravity, per-side attack handicaps (50-200%) and starting garbage.
- **Serious CPU opponents** — the AI plans like MisaMino / Cold Clear:
  a pathfinding move generator (soft-drop tucks, SRS spins), beam search
  over the preview queue, and an evaluation that tracks back-to-back,
  combos and T-spin-double setups — high-stage CPUs build and fire
  T-spins on their own.
- **Guideline-compliant mechanics**
  - [Super Rotation System (SRS)](https://tetrisch.github.io/main/srs.html) with full JLSTZ / I wall-kick tables
  - 7-bag randomizer, hold, 5-piece preview, ghost piece
  - Extended placement lockdown (0.5 s lock delay, 15 move resets)
  - T-Spin / T-Spin Mini detection (3-corner rule + TST kick exception)
  - Back-to-Back, combos, perfect clears, guideline scoring
  - Piece spawning in rows 21–22 with immediate drop, block-out / lock-out top-out rules
- **Configurable controls** — every action can be rebound in the Settings menu;
  DAS / ARR / SDF handling tuning and volume sliders included, plus gamepad
  support with a fixed layout. Settings persist to disk.
- **English / Japanese UI** — auto-detected from the OS language, switchable
  in the settings.
- **Flashy presentation** — HDR bloom on everything that matters, square
  glow particles, hard-drop light trails, line-clear light bars, shockwave
  frames, screen shake, banners, confetti, and rotating procedural
  background scenes (morphing 3D particle figures, matrix-style code rain,
  a spiral galaxy, an audio-reactive visualizer, plants, a Sierpinski
  carpet, a pixel sunset, Conway's Life, and a live piano roll of the
  music) that all pulse with the game's own soundscape
- **Generated music** — there are no music files. A four-voice NES-style
  synthesizer and an algorithmic composer write the BGM as you play: calm
  and bright in solo modes, fast and minor in versus, with the tempo
  stepping up as the stack climbs. See [`crates/chiptune`](crates/chiptune)
- **Audio** — CC0 8-bit sound effects by Juhani Junkala; combo chimes climb
  a pentatonic scale as the combo counter grows

## Download

Prebuilt binaries for macOS (Apple Silicon / Intel), Linux and Windows are
on the [Releases](https://github.com/uthree/bevytris/releases) page.
Extract the archive and run the `bevytris` binary — keep the bundled
`assets/` folder next to the executable.

## Building & running

Requires a recent stable Rust toolchain (edition 2024).

```bash
cargo run --release
```

The first build compiles Bevy and takes a few minutes; subsequent builds are fast.

Unit tests for the rules engine (SRS tables, scoring, T-spins, garbage):

```bash
cargo test -p bevytris-core
```

## Default controls

| Action      | Key         |
| ----------- | ----------- |
| Move left   | ←           |
| Move right  | →           |
| Soft drop   | ↓           |
| Hard drop   | Space       |
| Rotate CW   | ↑           |
| Rotate CCW  | Z           |
| Hold        | C           |
| Zone        | V           |
| Pause       | Esc         |
| Fullscreen  | F11         |

Menus: arrow keys + Enter (mouse also works). All gameplay keys can be rebound
in **SETTINGS**; press Enter on a binding row, then press the new key.

Settings are stored as RON at the platform config directory, e.g.
`~/Library/Application Support/bevytris/settings.ron` on macOS or
`~/.config/bevytris/settings.ron` on Linux.

## Architecture

- `crates/core` (`bevytris-core`) — engine-independent rules: board, pieces,
  SRS kicks, 7-bag, gravity/lockdown, scoring, garbage, the zone mechanic,
  and the CPU opponent (pathfinding movegen + beam search over a
  Dellacherie-style evaluation extended with attack and T-spin terms).
  Fully unit-tested; `cargo run -p bevytris-core --example selfplay
  --release` benchmarks the AI ladder headlessly.
- `crates/chiptune` — the music engine, also engine-independent: a
  nine-voice synthesizer (three PolyBLEP pulses, a VRC6-style sawtooth, a
  32-step wavetable, the 15-bit LFSR noise twice over, a channel of
  samples synthesized at startup, the NES's own nonlinear mixer, stereo
  panning) plus an algorithmic composer. The whole thing is one object:
  hand `Director` the gameplay parameters, ask it for samples.
  `cargo run -p bevytris-chiptune --release --example render --
  --profile vs --seed 42 --secs 60 --ramp` writes a WAV without launching
  the game, running exactly the code the game runs.

### Reproducing a piece of music

Every session's music comes from one seed, printed in the log at startup
and shown in the corner toast (`♪ A minor · 6/8 · 148 BPM · #c0ffee`).
Set `BEVYTRIS_MUSIC_SEED=0xc0ffee` to replay it note for note, or pass
the same value to `--seed` to render it offline. `BEVYTRIS_MUSIC_DEBUG=1`
puts a live readout on screen — profile, bar, key, mode, meter, kit,
which layers are audible, playhead, intensity — and logs every change of
piece.
- `src/` — the Bevy app: rendering, input (DAS/ARR), menus, settings
  persistence, particles/shake/banner effects, audio playback (CC0 sample
  banks, with runtime pitch-shifting for combo chimes) and the streaming
  bridge that feeds generated PCM to `bevy_audio`.

## Credits & licenses

See [assets/CREDITS.md](assets/CREDITS.md) for full details.

- **Code**: licensed under [Apache-2.0](LICENSE).
- **Music**: none shipped — the BGM is generated at runtime by this
  repository's own [`crates/chiptune`](crates/chiptune).
- **Sound effects**: ["512 Sound Effects (8-bit style)"](https://opengameart.org/content/512-sound-effects-8-bit-style)
  by **Juhani Junkala** (SubspaceAudio) — **CC0**. Trimmed and normalized.
- **Jingles** (tetris / T-spin / perfect clear phrases):
  ["Music Jingles"](https://kenney.nl/assets/music-jingles) by **Kenney** — **CC0**.
- **Background art**: ["Space Background"](https://opengameart.org/content/space-background-1)
  by **Westbeam** — **CC0/WTFPL**.
- **Font**: [Misaki Font (美咲フォント)](https://littlelimit.net/misaki.htm)
  © 2002-2021 Num Kadoma — an 8x8 Japanese pixel font distributed as free
  software ("unlimited permission ... with or without modification, either
  commercially or noncommercially", no warranty). Bevy's bundled
  [Fira Mono](https://github.com/mozilla/Fira) subset (SIL OFL 1.1) remains
  as the engine fallback.
- **Sound effects & other visuals**: generated procedurally by code in this
  repository (`src/audio.rs`, `src/effects.rs`).

This is a fan-made, non-commercial clone built for learning purposes.
*Tetris* is a trademark of Tetris Holding, LLC; this project is not
affiliated with or endorsed by Tetris Holding or The Tetris Company.
