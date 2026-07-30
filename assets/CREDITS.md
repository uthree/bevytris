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
  at runtime as combos climb. `assets/sfx/muffled/*.wav` are lowpassed
  (2x biquad at 750 Hz, soft-limited) copies of every sound, played while
  the zone super move stops time.
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

## Music

**None shipped.** The game has no music files at all. Every note of BGM is
generated while you play by `crates/chiptune` — an original four-voice
NES-style synthesizer and algorithmic composer written for this project,
covered by the repository's own license.

(Earlier releases used CC0 tracks by **Juhani Junkala** (SubspaceAudio) and
**SketchyLogic** from OpenGameArt. Those files were removed in 0.4.0; the
sound effects below are unaffected.)

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

## Character packs (`assets/characters/`)

> **These files are not under the repository's Apache-2.0 licence.**
> The code is Apache-2.0. A character pack is not, because it depicts
> characters this project does not own. See the terms below before reusing
> anything in this directory.

Characters are an optional drop-in folder: a name, portrait art and voice
clips, all cosmetic. Each pack carries its own `SOURCE.md` recording where
its art came from, and its `metadata.json` names the voice it borrowed.

### Voices — VOICEVOX

Synthesized with **[VOICEVOX](https://voicevox.hiroshiba.jp/)**, a free
Japanese text-to-speech engine, by `scripts/make_dummy_characters.sh`.

Audio generated from a VOICEVOX voice library may be used commercially and
non-commercially **provided the character is credited wherever the audio is
used**. Each pack writes its credit into `metadata.json`'s `author` field,
which the character picker displays on screen.

- `VOICEVOX:四国めたん`, `VOICEVOX:ずんだもん` — terms:
  <https://voicevox.hiroshiba.jp/term/> and
  <https://zunko.jp/con_ongen_kiyaku.html>

Note that the engine and the voice libraries themselves are **not**
redistributed here, and may not be; only audio generated with them.

### Character designs — 東北ずん子・ずんだもんプロジェクト

Where a pack depicts **ずんだもん** or another character of the
[東北ずん子・ずんだもんプロジェクト](https://zunko.jp/), the art is an
original drawing made for this project — a 二次創作 — and **not** a copy of
any official or fan-made illustration file. It is used under the project's
own guidelines: <https://zunko.jp/guideline.html>

What that means for anyone reusing it:

- **Non-commercial use only.** The guidelines permit derivative works by
  individual creators without prior approval for non-commercial use;
  commercial use requires a separate licence contract with the rights
  holder. bevytris is free and non-commercial, which is what makes this
  work — and is a constraint on this repository, not just on the art.
- No copyright notice is required by the guidelines. This section exists
  anyway, because a reader who assumed Apache-2.0 applied would be wrong.
- The prohibitions in the guidelines travel with the art: nothing that
  damages the characters' image, no political or religious use, no adult
  content, and no selling the images as products in themselves.

The rights holders' own AI position is why generated art is acceptable
here: the project **published a training-data pack in April 2023** so that
people could build their own models, and supplied data to the opt-in,
Fairly Trained-certified **Mitsua Likes** in December 2024. Generating
these characters is something the rights holders actively support, not
something they merely tolerate. Costume and colour changes are explicitly
allowed by the guidelines; the character has to stay recognisable.

### Art that depicts nobody else's characters

Original characters generated for this project carry the repository's own
licence. Open-weight image models place their conditions on the *weights*,
not the *outputs* — the Fair AI Public License and CreativeML OpenRAIL both
state that outputs are not covered and that no contributor claims rights in
them — and only outputs are redistributed here. (Generated images may not
be copyrightable in every jurisdiction; that costs this project nothing,
since it needs the right to distribute rather than the right to exclude.)

Placeholder portraits in the throwaway `alice` and `bob` fixtures are
procedural SVG from `scripts/make_dummy_characters.sh` and carry the
repository licence.

## Everything else

Particle/glow textures and other visuals, plus the synthesized
zone-activation boom (`sfx/zone_boom.wav`), are generated procedurally by
code from this repository and carry the repository license.
