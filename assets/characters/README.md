# Character packs

A **character** is an optional cosmetic pack: a name, some portrait art and a
handful of voice clips. Characters never affect gameplay — no stats, no
abilities. Everything except `metadata.json` is optional, and a pack with only
a `metadata.json` must still load.

Six ship with the game — `mint`, `rosa`, `amber`, `vio`, `cobalt` and `ash`.
`scripts/make_characters.sh` holds the whole cast (designs, prompts, voice
assignments) and rebuilds any of it. Their art is original and carries the
repository's licence; their voices are borrowed from VOICEVOX and credited.
See [`../CREDITS.md`](../CREDITS.md), and read it before adding a pack of
your own — the two halves answer to different terms.

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
  "display_name": "ミント",
  "ascii_name": "MINT",
  "flavor": "落ち着いていて、いつも音楽を聴いている。",
  "author": "VOICEVOX:四国めたん",
  "portrait_alpha": 0.11,
  "voice_gain": 1.0
}
```

| field | meaning |
| --- | --- |
| `schema` | Format version. Currently always `1`. A newer one loads anyway, with a warning: losing the whole character is worse than losing whatever the new field did. |
| `display_name` | Shown in game. May be non-ASCII — every shipped pack is, which is what exercises the Japanese font path. |
| `ascii_name` | ASCII fallback for the 8x8 pixel font. Max 24 characters. |
| `flavor` | One-line flavour text. |
| `author` | Shown in the picker under the focused character. This is where a pack states the credit it owes, which is why every shipped pack puts its VOICEVOX line here. |
| `portrait_alpha` | Opacity of the standing portrait behind the board. Clamped to `0.05..0.45`. **Dense art wants far less than the 0.20 default** — see below. |
| `voice_gain` | Per-character volume multiplier for the voice clips. Clamped to `0.2..2.0`. |

Both numeric fields are clamped rather than rejected, so an out-of-range value
degrades the look rather than dropping the character.

### On `portrait_alpha`

Start lower than you think. The default of `0.20` was chosen against sparse
placeholder art; a real illustration at that value competes with the falling
piece and wins. The six packs that ship use `0.11`.

The reason is not just coverage. The portrait is drawn through
`emissive(.., 1.15)`, which pushes it into HDR so it does not read as a grey
sticker against the bloom-boosted background scenes — and white pushed into
HDR *blooms*. A character in a white shirt is therefore much louder than the
alpha alone suggests, and the effect does not scale down as fast as the
number does. Check it against a real board rather than trusting the value.

## Which portrait goes where

`standing_right.png` shows the character **facing right**, so it belongs to the
**left-hand** board — the character looks in towards the middle of the screen.
`standing_left.png` is the mirror case for the **right-hand** board. Getting
this backwards makes both players look away from each other. `make_character_art.sh`
derives the left-facing art by mirroring the right, so a pack built with it
cannot get this wrong.

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

### Building the set from one drawing

Only `standing.png` is really worth drawing. `scripts/make_character_art.sh`
derives the rest:

```sh
scripts/make_character_art.sh --from ~/art/mychar mychar
```

It takes `standing.png` (required) plus `win.png`, `lose.png`, `cutin.png` and
`icon.png` if you have them, and writes the pack's `images/` — mirroring the
standing art for the other facing and cropping the face for the icon. Anything
you did supply is used as-is rather than derived, and anything that arrives
without an alpha channel has its background cut first. It also writes a
`SOURCE.md` recording where the art came from, which is what
`assets/CREDITS.md` asks for.

The cut-in is the one worth drawing separately rather than deriving. It is
swept full-bleed behind the boards at full opacity, so it wants the opposite
of a portrait: wide, close, and with a background of its own — which the
derivation cannot invent. Supply one and it is kept exactly as drawn,
background included.

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

## Rebuilding the cast

`scripts/make_characters.sh` holds all six — their designs, prompts, voice
assignments and flavour text — and rebuilds any of them:

```sh
scripts/make_characters.sh              # everything
scripts/make_characters.sh mint rosa    # just those
scripts/make_characters.sh --voices     # voices and metadata only
scripts/make_characters.sh --art        # art only
```

It needs a VOICEVOX ENGINE for the voices (it starts the one inside
`VOICEVOX.app` if it finds one, or set `VOICEVOX_URL`), a ComfyUI for the art
(`COMFYUI_URL`), and macOS `sips` to fit the images. Anything missing is
reported and skipped rather than failed.

Do not hand-edit a pack — edit the script. A pack is derived output, and the
only copy of *why* a character looks the way it does is in there.

## Testing the loader

Everything above is somebody else's input as far as the game is concerned:
the loader's contract is that a malformed pack is skipped with a warning
naming the folder and the reason, never a panic and never a path that escapes
the pack. `scripts/make_broken_characters.sh` writes packs designed to break
that — truncated JSON, a JPEG named `.png`, a text file named `.wav`,
out-of-range numbers, a display name full of bidi overrides, a symlink walking
out of the assets directory — into a scratch directory outside the repository:

```sh
scripts/make_broken_characters.sh /tmp/broken-characters
```
