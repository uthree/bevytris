# ComfyUI workflow templates

`scripts/make_character_art.sh` drives ComfyUI over its HTTP API. The workflows
it submits live here, one JSON file per kind of image, exported from ComfyUI
itself with **Save (API Format)** — not the ordinary Save, which writes the
editor's own graph format and is not what `POST /prompt` accepts.

## The contract

A template is an ordinary API-format workflow with placeholders written into
its string values. The script substitutes them and rejects the workflow if any
are left over, so a typo fails here rather than as a 400 from the far end.

| placeholder | in | becomes |
| --- | --- | --- |
| `__PROMPT__` | `standing.json` | the character's positive prompt |
| `__NEGATIVE__` | `standing.json` | the negative prompt |
| `__SEED__` | `standing.json` | the seed, as a number |
| `__WIDTH__` | `standing.json` | the generation width |
| `__HEIGHT__` | `standing.json` | the generation height |
| `__IMAGE__` | `matte.json` | the uploaded file's name in ComfyUI's input dir |
| `__MODEL__` | `matte.json` | `$MATTE_MODEL` |

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

Size does not have to be exact. The script scales and pads onto the documented
canvas afterwards, so a workflow that wants to generate at a resolution its
model likes should do that.

## The templates

- `standing.json` — a figure on a white background. Generation only; it does
  not cut the background, because the same cut has to be applied to art that
  arrives from somewhere else too, and doing it in one place is what makes
  `--from` behave the same as `--generate`.
- `matte.json` — the cut. Takes an image that has already been uploaded to
  ComfyUI's input directory (`POST /upload/image`, which `comfy_upload` does)
  and returns it with an alpha channel.

## About the matting node

`BiRefNetRMBG` comes from the **ComfyUI-RMBG** custom node pack. It is not
ComfyUI's own background-removal node, and it does not use core's
`models/background_removal/` folder — that folder stays empty with the pack
installed, which is confusing enough to be worth writing down.

The model matters. `BiRefNet_toonout` is the variant trained on illustration;
the photographic ones read a white shirt inside a black outline as more
background and cut a hole through it. `BiRefNet-HR-matting` erased one
character's shirt entirely. `$MATTE_MODEL` overrides the choice.

Doing this with a model rather than a flood fill is not fussiness. The flood
fill it replaced could only remove white connected to the canvas border, and
the poses defeat that on both counts: victory poses land on a coloured impact
splash, defeat poses in a cloud of dust, and what survives is a coloured slab
under the character's feet.

## Licensing

Outputs, not weights. Open-weight models put their conditions on the weights —
the Fair AI Public License and CreativeML OpenRAIL both say outputs are not
covered and that no contributor claims rights in them — and this repository
redistributes only outputs. Record the model in each pack's `SOURCE.md`, which
the script writes; `assets/CREDITS.md` explains why that matters.
