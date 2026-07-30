# ComfyUI workflow templates

`scripts/make_character_art.sh` drives ComfyUI over its HTTP API. The workflows
it submits live here, one JSON file per kind of image, exported from ComfyUI
itself with **Save (API Format)** — not the ordinary Save, which writes the
editor's own graph format and is not what `POST /prompt` accepts.

Nothing here yet. A template cannot be written blind: which nodes exist and
what the checkpoint is called are facts about the machine running ComfyUI, so
these get exported once that machine is up.

## The contract

A template is an ordinary API-format workflow with placeholders written into
its string values. The script substitutes them and rejects the workflow if any
are left over, so a typo fails here rather than as a 400 from the far end.

| placeholder | becomes |
| --- | --- |
| `__PROMPT__` | the character's positive prompt |
| `__SEED__` | the seed, as a number |
| `__WIDTH__` | 512 |
| `__HEIGHT__` | 1024 |

Substitution happens on the *parsed* document, so a prompt containing quotes or
backslashes is safe.

A placeholder that is the entire string and resolves to digits is emitted as a
number, so a KSampler's seed is written

```json
"seed": "__SEED__"
```

and arrives as an integer. The same trick covers width and height.

## What a workflow must do

**Save exactly one image.** The script takes the first image in the history
output and ignores the rest; a workflow with several `SaveImage` nodes has more
than one idea about what it is for, and choosing between them from outside would
be guessing. Split it into two templates instead.

**Produce an alpha channel if it can.** The portraits are composited over the
playfield, so a background that is merely white is a white rectangle on screen.
Either generate transparency directly (LayerDiffuse) or put a background
removal node — BiRefNet handles hair far better than the older U2Net default —
before the save. `make_character_art.sh` prints a warning for art that arrives
opaque, but it cannot fix it.

Size does not have to be exact. The script scales and pads onto the documented
canvas afterwards, so a workflow that wants to generate at a resolution its
model likes should do that.

## Suggested templates

- `standing.json` — the one required image: a full-body portrait, facing right.
  The left-facing version is a mirror and is never generated.
- `pose.json` — the win and lose poses. Almost certainly img2img from the
  standing art at a middling denoise rather than a fresh generation, because
  two independent generations of "the same" character are two characters.
- `cutin.json` — optional, wide (1024x512). Worth having: the derived fallback
  is the standing art centred on a wide transparent canvas, which is a small
  figure with a lot of nothing either side.

## Licensing

Outputs, not weights. Open-weight models put their conditions on the weights —
the Fair AI Public License and CreativeML OpenRAIL both say outputs are not
covered and that no contributor claims rights in them — and this repository
redistributes only outputs. Record the model in each pack's `SOURCE.md`, which
the script writes; `assets/CREDITS.md` explains why that matters.
