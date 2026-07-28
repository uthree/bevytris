# Third-party asset credits

## Sound effects (`assets/sfx/`)

**"The Essential Retro Video Game Sound Effects Collection [512 sounds]"**
(a.k.a. "512 Sound Effects (8-bit style)") by **Juhani Junkala** (SubspaceAudio)

- Source: https://opengameart.org/content/512-sound-effects-8-bit-style
- License: **CC0 1.0** (public domain dedication), as stated on the source page.
- Files: all `assets/sfx/*.wav` (27 sounds, renamed after their in-game role;
  e.g. `hard_drop.wav` is the pack's `sfx_exp_shortest_hard2.wav`).
- Modifications: mixed down to mono where needed, trailing silence trimmed,
  peak-normalized to -1 dBFS, re-encoded as 16-bit PCM WAV. The combo chime
  (`combo.wav`, from `sfx_coin_single3.wav`) is additionally pitch-shifted
  at runtime as combos climb.
- Although CC0 requires no attribution, we credit the author with pleasure.

## Music (`assets/music/`)

**"Retro Game Music Pack" (5 Chiptunes: Action)** by **Juhani Junkala** (SubspaceAudio)

- Source: https://opengameart.org/content/5-chiptunes-action
- License: **CC0 1.0** (public domain dedication) — confirmed in the pack's INFO.txt:
  "These music tracks have been released under CC0 creative commons license.
  You can do anything you want with these tunes."
- Files: `title.ogg` (Title Screen), `level1.ogg` (Level 1), `level2.ogg`
  (Level 2), `level3.ogg` (Level 3), `ending.ogg` (Ending)
- Modifications: converted from the original WAV files to OGG Vorbis;
  otherwise unchanged.
- Although CC0 requires no attribution, we credit the author with pleasure.
  Contact for commissions: juhani.junkala@musician.org

## Images (`assets/images/`)

**"Space Background"** by **Westbeam**

- Source: https://opengameart.org/content/space-background-1
- License: **CC0 / WTFPL** (author: "No GPL or CC-License. Published under
  the terms of the WTFPL.")
- File: `space_bg.png` (original `back_3.png`, unchanged)

## Font (`assets/fonts/`)

**Misaki Font (美咲フォント)** — 8x8 Japanese pixel font

- © 2002-2021 **Num Kadoma** (門真なむ)
- Source: https://littlelimit.net/misaki.htm
- File: `misaki_gothic_2nd.ttf` (美咲ゴシック第2, unchanged; embedded into
  the binary at build time and used as the game-wide default font)
- License (from the distribution): "These fonts are free software.
  Unlimited permission is granted to use, copy, and distribute them,
  with or without modification, either commercially or noncommercially.
  THESE FONTS ARE PROVIDED 'AS IS' WITHOUT WARRANTY."

Bevy's bundled default font (a **Fira Mono** subset © Mozilla Foundation,
**SIL OFL 1.1**) remains embedded in the engine as a fallback.

## Everything else

Particle/glow textures and other visuals are generated procedurally by code
in this repository and carry the repository license.
