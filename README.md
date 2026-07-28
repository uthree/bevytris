# bevytris

A guideline-flavored Tetris clone written in Rust with [Bevy Engine](https://bevy.org) 0.19.

## Features

- **Marathon mode** — classic single-player, guideline gravity curve, 10 lines per level
- **VS CPU mode** — battle a computer opponent (Easy / Normal / Hard) with
  garbage attack & cancellation rules modeled after modern versus games
- **Guideline-compliant mechanics**
  - [Super Rotation System (SRS)](https://tetrisch.github.io/main/srs.html) with full JLSTZ / I wall-kick tables
  - 7-bag randomizer, hold, 5-piece preview, ghost piece
  - Extended placement lockdown (0.5 s lock delay, 15 move resets)
  - T-Spin / T-Spin Mini detection (3-corner rule + TST kick exception)
  - Back-to-Back, combos, perfect clears, guideline scoring
  - Piece spawning in rows 21–22 with immediate drop, block-out / lock-out top-out rules
- **Configurable controls** — every action can be rebound in the Settings menu;
  DAS / ARR handling tuning and volume sliders included. Settings persist to disk.
- **Flashy presentation** — particles, screen shake, line-clear banners,
  full-screen flashes, confetti, starfield background
- **100% procedural audio** — every sound effect and both chiptune BGM loops are
  synthesized in code at startup; the game ships with zero audio asset files

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
| Pause       | Esc         |

Menus: arrow keys + Enter (mouse also works). All gameplay keys can be rebound
in **SETTINGS**; press Enter on a binding row, then press the new key.

Settings are stored as RON at the platform config directory, e.g.
`~/Library/Application Support/bevytris/settings.ron` on macOS or
`~/.config/bevytris/settings.ron` on Linux.

## Architecture

- `crates/core` (`bevytris-core`) — engine-independent rules: board, pieces,
  SRS kicks, 7-bag, gravity/lockdown, scoring, garbage, and the CPU opponent's
  placement search (Dellacherie-style evaluation). Fully unit-tested.
- `src/` — the Bevy app: rendering, input (DAS/ARR), menus, settings
  persistence, particles/shake/banner effects, and the procedural audio
  synthesizer (WAV generated in memory at startup).

## Credits & licenses

- **Code**: licensed under [Apache-2.0](LICENSE).
- **Font**: the UI uses Bevy's bundled default font, a subset of
  [Fira Mono](https://github.com/mozilla/Fira) © Mozilla Foundation, licensed
  under the [SIL Open Font License 1.1](https://openfontlicense.org/). It is
  embedded in the Bevy engine itself; no font files ship with this repository.
- **Audio**: all sound effects and music are original works generated
  procedurally by code in this repository (`src/audio.rs`). No third-party
  audio assets are used. The BGM tracks are original compositions written for
  this project.
- **Graphics**: all visuals are solid-color sprites and text drawn by code;
  no third-party art assets are used.

This is a fan-made, non-commercial clone built for learning purposes.
*Tetris* is a trademark of Tetris Holding, LLC; this project is not
affiliated with or endorsed by Tetris Holding or The Tetris Company.
