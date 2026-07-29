# Character packs

A **character** is an optional cosmetic pack: a name, some portrait art and a
handful of voice clips. Characters never affect gameplay — no stats, no
abilities. Everything except `metadata.json` is optional, and a pack with only
a `metadata.json` must still load.

Each pack is one directory under `assets/characters/`:

```
assets/characters/<id>/
├── metadata.json                    required
├── images/
│   ├── standing_right.png           512x1024, character faces RIGHT
│   ├── standing_left.png            512x1024, character faces LEFT
│   └── cutin.png                    1024x512, optional, wide
└── voices/
    └── <kind>.wav                   all optional, see the table below
```

**The directory name is the id.** There is deliberately no `id` field in
`metadata.json`, so the id cannot disagree with where the pack lives. Keep it
to `[a-z0-9_]`: it ends up in config files and log lines, and directories
beginning with `.` are skipped as hidden.

## metadata.json

```json
{
  "schema": 1,
  "display_name": "ダミー・ボブ",
  "ascii_name": "DUMMY BOB",
  "flavor": "テスト用のダミーキャラ。日本語をしゃべる。",
  "author": "scripts/make_dummy_characters.sh",
  "portrait_alpha": 0.20,
  "voice_gain": 1.0
}
```

| field | meaning |
| --- | --- |
| `schema` | Format version. Currently always `1`. A pack with an unknown `schema` should be skipped with a warning, not loaded optimistically. |
| `display_name` | Shown in game. May be non-ASCII; `ダミー・ボブ` exists precisely to exercise the Japanese font path. |
| `ascii_name` | ASCII fallback for the 8x8 pixel font. Max 24 characters. |
| `flavor` | One-line flavour text. |
| `author` | Who made the pack. |
| `portrait_alpha` | Opacity of the standing portrait behind the board. Clamped to `0.05..0.45`. |
| `voice_gain` | Per-character volume multiplier for the voice clips. Clamped to `0.2..2.0`. |

Both numeric fields are clamped rather than rejected, so an out-of-range value
degrades the look rather than dropping the character.

## Which portrait goes where

`standing_right.png` shows the character **facing right**, so it belongs to the
**left-hand** board — the character looks in towards the middle of the screen.
`standing_left.png` is the mirror case for the **right-hand** board. Getting
this backwards makes both players look away from each other, which is why the
dummy art has a large arrow printed on it.

The portraits have a transparent margin, so they must be composited with alpha,
not drawn as opaque quads.

## Voice kinds

Filenames are fixed. Anything else in `voices/` is ignored. A missing file just
means that event has no line.

| file | English | Japanese |
| --- | --- | --- |
| `ready.wav` | Here we go | いくよー |
| `clear.wav` | Nice | いいね |
| `tetris.wav` | Tetris! | テトリス！ |
| `tspin.wav` | T spin! | ティースピン！ |
| `perfect_clear.wav` | Perfect clear! | パーフェクトクリア！ |
| `combo.wav` | Combo! | コンボ！ |
| `attack.wav` | Take this | くらえー |
| `damage.wav` | Ouch | いたっ |
| `zone_start.wav` | Zone, activate | ゾーン、発動 |
| `zone_finish.wav` | How was that | どうだった |
| `win.wav` | I win! | わたしの勝ち！ |
| `lose.wav` | No way | そんなー |

### Audio format

Mono 16-bit little-endian PCM in a RIFF/WAVE container. That is what the
`wav` feature of Bevy's audio backend decodes. Anything else — MP3, IEEE
float samples, an AIFF container with a `.wav` extension — will not play.

Recording one with the macOS `say` command (the `--data-format` flag is
mandatory; `.aiff` output does not work):

```sh
say -v Samantha -o ready.wav --data-format=LEI16@22050 "Here we go"
say -v Kyoko    -o ready.wav --data-format=LEI16@22050 "いくよー"
```

`say` writes a `JUNK` padding chunk before `fmt `, which is legal RIFF and is
skipped correctly by the decoder — but it does mean `fmt ` is not at byte 12.
Do not hand-write a parser that assumes it is.

## Art notes

Standing art is 512x1024 and cut-in art is 1024x512. Both are PNG with an
alpha channel. Keep the character roughly centred horizontally in the standing
art; the board is drawn over the middle of it.

## The dummy packs

`alice` and `bob` are generated placeholders, **not** real content:

* `alice` — `DUMMY ALICE`, English (Samantha), teal.
* `bob` — `ダミー・ボブ`, Japanese (Kyoko), magenta. Non-ASCII display name on
  purpose, so the pixel-font and `ascii_name` fallback paths get exercised.

They are listed in `.gitignore` and are **not** committed. Regenerate them with:

```sh
scripts/make_dummy_characters.sh
```

Do not hand-edit anything under `assets/characters/alice/` or
`assets/characters/bob/` — edit the script. On a machine without `sips` and
`say` the script prints a notice and exits 0, so the packs are simply absent;
the game must cope with that.

For negative-path testing there is a second mode that writes deliberately
broken packs (malformed JSON, a JPEG named `.png`, out-of-range numbers, a
symlink escaping the pack, illegal directory names, and so on) into a scratch
directory outside the repository:

```sh
scripts/make_dummy_characters.sh --broken /tmp/broken-characters
```
