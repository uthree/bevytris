# Third-party asset credits

## Sound effects (`assets/sfx/`)

**"The Essential Retro Video Game Sound Effects Collection [512 sounds]"**
(a.k.a. "512 Sound Effects (8-bit style)") by **Juhani Junkala** (SubspaceAudio)

- Source: https://opengameart.org/content/512-sound-effects-8-bit-style
- License: **CC0 1.0** (public domain dedication), as stated on the source page.
- Files: all `assets/sfx/*.wav` (27 sounds, renamed after their in-game role;
  e.g. `hard_drop.wav` is the pack's `sfx_exp_shortest_hard2.wav`).
- Modifications: mixed down to mono where needed, trailing silence trimmed,
  peak-normalized to -1 dBFS, re-encoded as 16-bit PCM WAV. The movement and
  drop sounds (`move.wav`, `soft_drop.wav`, `hard_drop.wav`) are steeply
  high-passed (2.5-3 kHz) to sit light in the mix. The combo chime
  (`combo.wav`, from `sfx_coin_single3.wav`) is additionally pitch-shifted
  at runtime as combos climb.
- Although CC0 requires no attribution, we credit the author with pleasure.

**"Impact Sounds"** by **Kenney** (kenney.nl)

- Source: https://kenney.nl/assets/impact-sounds
- License: **CC0 1.0** (public domain dedication).
- Files (all derived from `impactBell_heavy_002.ogg`): `phrase_tetris.wav`
  and `phrase_tspin.wav` (rising major-chord arpeggios mixed from
  pitch-shifted copies of the tuned bell hit), `clear_note.wav` (the
  tetris arpeggio's first bell note isolated — its real attack extended
  with a partial-resynthesis ring-out), `phrase_perfect.wav` (two-octave
  rising bell arpeggio into a high chord over a sub-octave bell, mixed
  from pitch-shifted copies of that same note), and `zone_ready.wav`
  (two-note rising chime — octave, then octave+5th — from the same bell).

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

**"4 Chiptunes (Adventure)"** by **Juhani Junkala** (SubspaceAudio)

- Source: https://opengameart.org/content/4-chiptunes-adventure
- License: **CC0 1.0** (public domain dedication) — confirmed in the pack's
  INFO.txt: "These music tracks have been released under CC0 creative
  commons license. You can do anything you want with these tunes."
- Files: `stage1.ogg` (Stage 1), `stage2.ogg` (Stage 2), `boss_fight.ogg`
  (Boss Fight), `stage_select.ogg` (Stage Select)
- Modifications: renamed only (the pack ships OGG Vorbis).

**"NES Shooter Music (5 tracks, 3 jingles)"** by **SketchyLogic**

- Source: https://opengameart.org/content/nes-shooter-music-5-tracks-3-jingles
- License: **CC0 1.0** (public domain dedication), as stated on the source
  page ("Attribution is completely optional").
- Files: `venus.wav` (Venus), `map.wav` (Map), `mars.wav` (Mars),
  `mercury.wav` (Mercury) — the pack's remaining tracks and jingles are
  not shipped.
- Modifications: renamed only.

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

Particle/glow textures and other visuals, plus the synthesized
zone-activation boom (`sfx/zone_boom.wav`), are generated procedurally by
code from this repository and carry the repository license.
