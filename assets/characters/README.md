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
│   ├── cutin.png                    optional; the big-move cut-in, drawn
│   │                                behind the boards as scenery
│   ├── icon.png                     optional, square-ish; the picker tile
│   ├── win.png                      optional; pose held on a win
│   └── lose.png                     optional; pose held on a loss
└── voices/
    ├── <kind>.wav                   all optional, see the table below
    └── count_01.wav … count_20.wav  all optional; the spoken numbers
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
| `schema` | Format version. Currently always `1`. A newer one loads anyway, with a warning: losing the whole character is worse than losing whatever the new field did. |
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

Only `standing_right.png` and `standing_left.png` matter for whether a pack
loads at all: a character with neither is skipped, because it would have nothing
to show on the board. A pack that ships only one facing gets it reused on both
sides.

The other three are pure extras and each falls back:

| file | used for | falls back to |
| --- | --- | --- |
| `cutin.png` | drawn large *behind* the boards on a TETRIS, T-Spin Double/Triple, Perfect Clear, a counter cancelling 4+ rows, or a zone release clearing 5+ lines, washed with that move's colour | the standing art |
| `icon.png` | the character picker tile | the standing art |
| `win.png` / `lose.png` | the pose held while the result is up, drawn *over* the field (the standing art sits behind the stack; this does not) next to the WIN/LOSE lettering, and again at the edges of the match-end result screen | the other one, then the standing art |

`standing_right.png` also stands at full height beside the character picker,
showing who the cursor is on.

Aspect ratio is preserved everywhere the art is drawn as UI (the cut-in strip
and the picker tile measure the image and scale it by height), so these do not
have to be any particular size. The board portrait is fitted to the field, so
very wide art ends up small rather than spilling out of the frame.

## Voice kinds

Filenames are fixed. Anything else in `voices/` is ignored. A missing file just
means that event has no line.

| file | when | English | Japanese |
| --- | --- | --- | --- |
| `select.wav` | confirmed in the picker | Leave it to me | まかせて |
| `ready.wav` | the countdown before a round | Here we go | いくよー |
| `clear.wav` | an ordinary line clear | Nice | いいね |
| `tetris.wav` | four lines at once | Tetris! | テトリス！ |
| `tspin.wav` | any T-spin | T spin! | ティースピン！ |
| `perfect_clear.wav` | the board left empty | Perfect clear! | パーフェクトクリア！ |
| `combo.wav` | a streak, when the numbers below are absent | Combo! | コンボ！ |
| `attack.wav` | a clear that sent garbage | Take this | くらえー |
| `counter.wav` | an attack that cancelled *all* the incoming garbage | Not today! | そうはさせない！ |
| `damage.wav` | 2–3 rows of garbage rose | Ouch | いたっ |
| `damage_heavy.wav` | 4+ rows rose at once | That is a lot | うわ、多い！ |
| `pinch.wav` | the stack got close to the top | This is bad | まずい、まずい |
| `zone_ready.wav` | the zone gauge filled | Zone, ready | ゾーン、いけるよ |
| `zone_start.wav` | the zone fired | Zone, activate | ゾーン、発動 |
| `zone_finish.wav` | the zone released its banked lines | How was that | どうだった |
| `win.wav` | won the round or the match | I win! | わたしの勝ち！ |
| `lose.wav` | lost the round or the match | No way | そんなー |

Taking garbage is the one event with two lines: two or three rows play
`damage.wav`, four or more play `damage_heavy.wav`. A pack that ships only
`damage.wav` uses it for both rather than going quiet on the big hits.
`counter.wav` falls back to `attack.wav` and `select.wav` to `ready.wav` the
same way, so a pack written before those existed keeps reacting.

`pinch.wav` and `zone_ready.wav` deliberately have no fallback: they describe a
state rather than an event, and an older line borrowed for them would fire at a
moment it does not fit.

### Counting

`count_01.wav` … `count_20.wav` are a character counting: *one, two, three*.
They are used twice —

* each clear of a combo, from the second one (so a five-chain counts
  `two, three, four, five`);
* each line a zone banks, as the total climbs.

Each number cuts off the one before it, and anything that actually matters — a
tetris, a hit — cuts off the number. Past twenty the counting simply stops.

A pack with no numbers is not worse off: a combo falls back to `combo.wav`
(rationed, so it does not repeat every clear), and a zone stays quiet until it
releases. Zero-pad the filenames; `count_3.wav` is ignored, though the loader
will point out that it looks like a typo for `count_03.wav`.

### Audio format

Mono 16-bit little-endian PCM in a RIFF/WAVE container. That is what the
`wav` feature of Bevy's audio backend decodes. Anything else — MP3, IEEE
float samples, an AIFF container with a `.wav` extension — will not play.

Sample rate is up to you; the dummy packs are 24 kHz.

Beware of padding chunks. macOS `say`, for one, writes a `JUNK` block before
`fmt `, which is legal RIFF and is skipped correctly by the decoder — but it
does mean `fmt ` is not at byte 12. Do not hand-write a parser that assumes
it is.

### Synthesising with VOICEVOX

[VOICEVOX](https://voicevox.hiroshiba.jp/) is a free Japanese text-to-speech
engine, and it is what the dummy packs are voiced with. It runs as a local
HTTP server — start `VOICEVOX.app`, or

```sh
docker run --rm -p 50021:50021 voicevox/voicevox_engine:cpu-latest
```

Two calls per clip. `/audio_query` turns text into a prosody document,
`/synthesis` renders it, and the result is already the exact WAV format
above:

```sh
curl -s -G --data-urlencode "text=いくよー" --data-urlencode "speaker=3" \
  -X POST http://127.0.0.1:50021/audio_query -o q.json
curl -s -X POST -H 'Content-Type: application/json' --data-binary @q.json \
  "http://127.0.0.1:50021/synthesis?speaker=3" -o ready.wav
```

`speaker` is a *style* id, not a character id — one character usually has
several. `curl -s http://127.0.0.1:50021/speakers` lists every one.

**Credit is not optional.** Audio generated from a VOICEVOX voice library
may be used commercially and non-commercially, but only if the character is
named wherever it is used — `VOICEVOX:ずんだもん`, and one line per character
you used. Put it in `metadata.json`'s `author` field: the picker shows that
under the focused character, so the credit ships with the pack instead of
depending on somebody remembering to write it down elsewhere.

The precise terms differ per character. Read the one you are using:
`curl -s "http://127.0.0.1:50021/speaker_info?speaker_uuid=<uuid>" | jq -r .policy`
prints it, and the character's page under
<https://voicevox.hiroshiba.jp/> links the full document. A few characters
add conditions — 青山龍星, for instance, requires prior contact if a company
is involved.

## Art notes

Standing art is 512x1024 and cut-in art is 1024x512. Both are PNG with an
alpha channel. Keep the character roughly centred horizontally in the standing
art; the board is drawn over the middle of it.

## The dummy packs

`alice` and `bob` are generated placeholders, **not** real content:

* `alice` — `DUMMY ALICE`, teal, voiced by `VOICEVOX:四国めたん`.
* `bob` — `ダミー・ボブ`, magenta, voiced by `VOICEVOX:ずんだもん`. Non-ASCII
  display name on purpose, so the pixel-font and `ascii_name` fallback paths
  get exercised.

The art is deliberately ugly procedural SVG that states its own id, facing
and pixel size, so a portrait drawn on the wrong side or at the wrong scale
says so out loud. The voices are real, because there is no such thing as
placeholder audio you can hear a bug in.

They are listed in `.gitignore` and are **not** committed. Regenerate them with:

```sh
scripts/make_dummy_characters.sh
```

Do not hand-edit anything under `assets/characters/alice/` or
`assets/characters/bob/` — edit the script. It needs `sips` (macOS) for the
art and a VOICEVOX ENGINE for the voices; it starts the one inside
`VOICEVOX.app` if it finds it, honours `VOICEVOX_URL` if you are running your
own, and prints a notice and exits 0 if neither is there. The packs are then
simply absent, which the game must cope with.

For negative-path testing there is a second mode that writes deliberately
broken packs (malformed JSON, a JPEG named `.png`, out-of-range numbers, a
symlink escaping the pack, illegal directory names, and so on) into a scratch
directory outside the repository:

```sh
scripts/make_dummy_characters.sh --broken /tmp/broken-characters
```
