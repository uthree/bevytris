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
clips, all cosmetic. Six ship with the game. Each pack carries its own
`SOURCE.md` recording where its art came from, and its `metadata.json`
names the voice it borrowed.

The two halves of a pack answer to different licences, which is the single
most confusable thing here: **the art is ours, the voices are borrowed.**

### Voices — VOICEVOX

Synthesized with **[VOICEVOX](https://voicevox.hiroshiba.jp/)**, a free
Japanese text-to-speech engine.

Audio generated from a VOICEVOX voice library may be used commercially and
non-commercially **provided the character is credited wherever the audio is
used**. Each pack writes its credit into `metadata.json`'s `author` field,
which the character picker displays on screen.

| pack | voice |
| --- | --- |
| `mint` | `VOICEVOX:四国めたん` |
| `rosa` | `VOICEVOX:春日部つむぎ` |
| `amber` | `VOICEVOX:ずんだもん` |
| `vio` | `VOICEVOX:冥鳴ひまり` |
| `cobalt` | `VOICEVOX:九州そら` |
| `ash` | `VOICEVOX:東北きりたん` |

Terms: <https://voicevox.hiroshiba.jp/term/>, and per character via
`curl "$VOICEVOX_URL/speaker_info?speaker_uuid=..."`, which returns each
library's own policy text. Some characters attach further conditions — one
requires prior contact where a company is involved — so a pack that changes
its voice must re-read them rather than assume.

Note that the engine and the voice libraries themselves are **not**
redistributed here, and may not be; only audio generated with them.

### Art — original, and under this repository's licence

The six characters — `mint`, `rosa`, `amber`, `vio`, `cobalt` and `ash` —
are **original designs made for this project**. They depict nobody else's
characters, so nobody else's terms apply to them, and the art carries the
repository's own licence. Each pack's `SOURCE.md` records how it was made;
`scripts/make_characters.sh` holds the cast and can rebuild all of it.

The art is generated with an open-weight image model. That is licensable
because such models place their conditions on the *weights*, not the
*outputs* — the Fair AI Public License and CreativeML OpenRAIL both state
that outputs are not covered and that no contributor claims rights in them
— and only outputs are redistributed here. (Generated images may not be
copyrightable in every jurisdiction. That costs this project nothing: it
needs the right to distribute, not the right to exclude. It does mean the
art cannot be claimed as anyone's exclusive property.)

Originality was the point rather than an accident. Adopting an existing
character would have meant working under that character's own terms —
for the obvious candidates, non-commercial only, with a licence contract
required otherwise — and pinning the whole repository to that condition
forever. **Anyone adding a pack that depicts someone else's character
should set `CHARACTER_DESIGN` when running `make_character_art.sh`**, which
writes the correct, more restrictive notice into that pack's `SOURCE.md`.
The terms for the characters whose voices are used here are at
<https://zunko.jp/guideline.html>; note that they are *not* the same as the
terms for the voices, which is the easiest thing in this whole area to get
wrong.

Placeholder portraits in the throwaway `alice` and `bob` fixtures are
procedural SVG from `scripts/make_dummy_characters.sh` and likewise carry
the repository licence.

## Everything else

Particle/glow textures and other visuals, plus the synthesized
zone-activation boom (`sfx/zone_boom.wav`), are generated procedurally by
code from this repository and carry the repository license.
