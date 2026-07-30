#!/usr/bin/env bash
#
# make_character_art.sh — build a character pack's images/ from generated art.
#
# WHAT THIS IS FOR
#   A character pack needs six images (see assets/characters/README.md).  Only
#   one or two of them are worth generating: the rest are the same art flipped,
#   cropped or re-canvassed, and deriving those here rather than generating
#   them separately is what keeps a character looking like one character.
#
#   So the contract is: you supply *hero* images, this supplies the pack.
#
#     hero            derived from it            why not generate it
#     -------------   ------------------------   ---------------------------
#     standing.png    standing_right.png         —
#                     standing_left.png          a mirror is the same person
#                     icon.png                   a crop of their own face
#                     cutin.png                  the same art, wide canvas
#     win.png         win.png                    a real pose change; generate
#     lose.png        lose.png                   a real pose change; generate
#     cutin.png       cutin.png                  optional; better if drawn wide
#
#   Everything but standing.png is optional, because the game already falls
#   back: no win.png means the standing art is held at the result screen, no
#   cutin.png means the standing art is what sweeps behind the boards.  A pack
#   built from exactly one image is a legitimate pack.
#
# WHY COMFYUI
#   Because it has an HTTP API, and that is the whole reason.  This repository
#   keeps its assets regenerable from a script — the sound effects, the music,
#   the placeholder portraits — and an asset you can only make by clicking
#   around in an app is an asset nobody can reproduce, including you in six
#   months.  ComfyUI takes a workflow as JSON and hands back the file, so the
#   prompt, the model and the seed can live in version control next to the
#   code that uses them.  Each pack gets a SOURCE.md recording exactly that.
#
#   It also does not care what hardware it runs on.  $COMFYUI_URL points this
#   at a machine with a real GPU; nothing else in here changes.
#
# ON THE LICENSING, SINCE THAT IS WHY WE ARE HERE
#   Open-weight image models put their conditions on the *weights*, not the
#   *outputs* — the Fair AI Public License and CreativeML OpenRAIL both state
#   that outputs are not covered and that no contributor claims rights in
#   them.  We redistribute outputs and never weights, so generated art can
#   ship under this repository's own license.  That is the point of doing it
#   this way: the character illustrations these packs would otherwise use are
#   third-party assets whose terms allow use in a finished work but not
#   redistribution of the files, which is exactly what committing them here
#   would be.
#
#   (Generated images may not be copyrightable in every jurisdiction.  That
#   costs us nothing: we need the right to distribute, not the right to stop
#   anyone else.  It does mean we cannot claim the art as ours.)
#
# USAGE
#   scripts/make_character_art.sh --check
#       Report what is available and what is missing, and exit.  Useful while
#       the ComfyUI end is still being built.
#
#   scripts/make_character_art.sh --from <dir> <id>
#       Skip generation.  Take <dir>/standing.png (and win.png, lose.png,
#       cutin.png, icon.png if present) and derive the pack into
#       assets/characters/<id>/images/.
#
#   scripts/make_character_art.sh <id>
#       Generate the hero images through ComfyUI, then derive as above.
#       Needs a workflow template; scripts/comfyui/README.md says what one
#       has to contain. Not wired up until there is one to drive it with.
#
# REQUIREMENTS
#   macOS `sips` for the derivation, python3 for JSON handling, and — for
#   generation only — a reachable ComfyUI.  Missing pieces are reported and
#   skipped rather than failed, the same way make_dummy_characters.sh does it.
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Locate ourselves.  Resolved from the script path so $PWD cannot change what
# this writes, matching make_dummy_characters.sh.
# ---------------------------------------------------------------------------
self="${BASH_SOURCE[0]}"
while [ -L "$self" ]; do
  link_target="$(readlink "$self")"
  case "$link_target" in
    /*) self="$link_target" ;;
    *) self="$(dirname "$self")/$link_target" ;;
  esac
done
SCRIPT_DIR="$(cd "$(dirname "$self")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
CHARACTERS_DIR="$REPO_ROOT/assets/characters"
WORKFLOW_DIR="$SCRIPT_DIR/comfyui"

if [ ! -f "$REPO_ROOT/Cargo.toml" ] || [ ! -d "$REPO_ROOT/assets" ]; then
  echo "make_character_art.sh: $REPO_ROOT does not look like the bevytris repo" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Sizes.  These are what assets/characters/README.md documents and what the
# game's loader checks, so they are not free parameters.
# ---------------------------------------------------------------------------
STANDING_W=512
STANDING_H=1024
CUTIN_W=1024
CUTIN_H=512
ICON=256

# Where the face sits in a standing portrait, as a fraction of its height.
# The icon is a square crop centred there rather than on the middle of the
# canvas, which on a full-body portrait would be somebody's waist.
FACE_CENTER=0.18

COMFYUI_URL="${COMFYUI_URL:-http://127.0.0.1:8188}"

# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------
HAVE_SIPS=1
HAVE_PYTHON=1
command -v sips >/dev/null 2>&1 || HAVE_SIPS=0
command -v python3 >/dev/null 2>&1 || HAVE_PYTHON=0

comfy_up() {
  curl -sS -m 5 "$COMFYUI_URL/system_stats" >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# Image derivation
#
# Every step is `sips`, which preserves the alpha channel and is the one
# image tool a Mac is guaranteed to have.  Note the argument order: sips takes
# HEIGHT then WIDTH, everywhere, which is backwards from how everyone says it
# out loud and is worth reading twice before editing.
# ---------------------------------------------------------------------------

# fit_into <src> <dst> <width> <height>
# Scale to fit inside the box without distorting, then centre it on a
# transparent canvas of exactly that size.  Two steps because -Z alone leaves
# whatever aspect the source had, and the loader is fussy about neither — but
# the game's own fitting code is simpler to reason about when every pack is
# the documented size.
fit_into() {
  local src="$1" dst="$2" w="$3" h="$4"
  local tmp="$dst.fit.png"
  # -Z scales the longest side, so ask for whichever side binds.
  sips -Z "$([ "$w" -gt "$h" ] && echo "$w" || echo "$h")" "$src" --out "$tmp" >/dev/null
  # -p pads to a canvas and fills with transparent, which is what we want and
  # is also the only thing it will do: --padColor rejects an 8-digit RGBA.
  sips -p "$h" "$w" "$tmp" --out "$dst" >/dev/null
  /bin/rm -f "$tmp"
}

# flip_h <src> <dst>
flip_h() {
  sips --flip horizontal "$1" --out "$2" >/dev/null
}

# png_crop <src> <dst> <x> <y> <w> <h>
#
# Crop an exact rectangle. Not `sips -c`: that crops from the centre, which on
# a full-body portrait is somebody's waist, and its `--cropOffset` escape
# hatch is undocumented and does not behave linearly — a -256 offset moves the
# window 512 pixels, and positive offsets are ignored outright. Rather than
# encode a quirk that a macOS update is free to change, this does the crop
# itself: zlib and struct are stdlib, and a rectangle out of an RGBA PNG is
# not much code.
#
# Rows outside the source come out fully transparent, so a crop may safely run
# past an edge.
png_crop() {
  python3 - "$@" <<'PY'
import struct, sys, zlib

src, dst, x0, y0, cw, ch = sys.argv[1], sys.argv[2], *map(int, sys.argv[3:7])

blob = open(src, 'rb').read()
w, h = struct.unpack('>II', blob[16:24])
depth, color = blob[24], blob[25]
if depth != 8 or color not in (2, 6):
    sys.exit(f"{src}: only 8-bit RGB/RGBA PNGs can be cropped here")
bpp = 4 if color == 6 else 3

at, idat = 8, b''
while at < len(blob):
    length = struct.unpack('>I', blob[at:at + 4])[0]
    if blob[at + 4:at + 8] == b'IDAT':
        idat += blob[at + 8:at + 8 + length]
    at += 12 + length

# Undo the per-row filters. Every PNG encoder picks these adaptively, so all
# five have to be handled even for an image that was written flat.
raw = zlib.decompress(idat)
stride = w * bpp
rows, prev, pos = [], bytearray(stride), 0
for _ in range(h):
    ftype = raw[pos]; pos += 1
    line = bytearray(raw[pos:pos + stride]); pos += stride
    for i in range(stride):
        a = line[i - bpp] if i >= bpp else 0
        b = prev[i]
        c = prev[i - bpp] if i >= bpp else 0
        if ftype == 1:   line[i] = (line[i] + a) & 255
        elif ftype == 2: line[i] = (line[i] + b) & 255
        elif ftype == 3: line[i] = (line[i] + (a + b) // 2) & 255
        elif ftype == 4:
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    rows.append(bytes(line))
    prev = line

blank = bytes(cw * 4)
out = bytearray()
for y in range(y0, y0 + ch):
    out.append(0)  # filter: none
    if not (0 <= y < h):
        out += blank
        continue
    line = rows[y]
    for x in range(x0, x0 + cw):
        if 0 <= x < w:
            px = line[x * bpp:(x + 1) * bpp]
            out += px if bpp == 4 else px + b'\xff'
        else:
            out += b'\0\0\0\0'

def chunk(tag, data):
    body = tag + data
    return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))

with open(dst, 'wb') as f:
    f.write(b'\x89PNG\r\n\x1a\n')
    f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', cw, ch, 8, 6, 0, 0, 0)))
    f.write(chunk(b'IDAT', zlib.compress(bytes(out), 6)))
    f.write(chunk(b'IEND', b''))
PY
}

# crop_face <src> <dst> <size>
# A square crop around where a face is, scaled to <size>.
crop_face() {
  local src="$1" dst="$2" size="$3"
  local sw sh side top tmp="$dst.crop.png"
  sw="$(png_width "$src")"
  sh="$(png_height "$src")"
  # As wide as the picture and as tall as it is wide, so the whole head stays
  # in frame whatever aspect the source had.
  side="$sw"
  [ "$side" -gt "$sh" ] && side="$sh"
  top="$(python3 -c "print(max(0, round($sh * $FACE_CENTER - $side / 2)))")"
  png_crop "$src" "$tmp" 0 "$top" "$side" "$side"
  sips -z "$size" "$size" "$tmp" --out "$dst" >/dev/null
  /bin/rm -f "$tmp"
}

# png_width / png_height <file> — straight out of the IHDR, no dependencies.
png_width() { python3 -c "import struct,sys;print(struct.unpack('>I', open(sys.argv[1],'rb').read(20)[16:20])[0])" "$1"; }
png_height() { python3 -c "import struct,sys;print(struct.unpack('>I', open(sys.argv[1],'rb').read(24)[20:24])[0])" "$1"; }

# check_png <file> <label>
# The same checks the game's loader makes, run here so a bad image is caught
# while somebody is looking at a terminal rather than at a missing character.
check_png() {
  local f="$1" label="$2"
  python3 - "$f" "$label" <<'PY'
import struct, sys
path, label = sys.argv[1], sys.argv[2]
head = open(path, 'rb').read(26)
if head[:8] != bytes([0x89, 80, 78, 71, 13, 10, 26, 10]) or head[12:16] != b'IHDR':
    sys.exit(f"{label}: not a PNG")
w, h = struct.unpack('>II', head[16:24])
if not (0 < w <= 4096 and 0 < h <= 4096):
    sys.exit(f"{label}: {w}x{h} is outside 1..=4096 per side")
if head[25] not in (4, 6):
    print(f"  note: {label} has no alpha channel — it will draw as a solid rectangle")
PY
}

# derive_pack <id> <hero-dir>
# Turn hero images into the pack's images/ directory.
derive_pack() {
  local id="$1" src="$2"
  local out="$CHARACTERS_DIR/$id/images"
  local standing="$src/standing.png"

  if [ ! -f "$standing" ]; then
    echo "make_character_art.sh: $standing is required and is not there" >&2
    return 1
  fi
  check_png "$standing" "standing.png"

  mkdir -p "$out"
  echo "  deriving into assets/characters/$id/images/"

  fit_into "$standing" "$out/standing_right.png" "$STANDING_W" "$STANDING_H"
  # The left-hand board's character faces right and the right-hand board's
  # faces left, so they look at each other across the screen.  One drawing
  # serves both: a mirrored portrait is the same character, and generating a
  # second one is how you end up with two people who nearly match.
  flip_h "$out/standing_right.png" "$out/standing_left.png"
  echo "    standing_right.png, standing_left.png (mirrored)"

  local pose
  for pose in win lose; do
    if [ -f "$src/$pose.png" ]; then
      check_png "$src/$pose.png" "$pose.png"
      fit_into "$src/$pose.png" "$out/$pose.png" "$STANDING_W" "$STANDING_H"
      echo "    $pose.png"
    else
      # Not an omission worth warning about: the game holds the standing art
      # at the result screen instead, which is a pack with less to say rather
      # than a pack that is broken.
      echo "    $pose.png — absent, the standing art will be held instead"
    fi
  done

  if [ -f "$src/cutin.png" ]; then
    check_png "$src/cutin.png" "cutin.png"
    fit_into "$src/cutin.png" "$out/cutin.png" "$CUTIN_W" "$CUTIN_H"
    echo "    cutin.png"
  else
    # A tall portrait on a wide canvas: the figure ends up centred and small,
    # which is honest but not much of a cut-in.  Worth drawing properly if the
    # character earns it.
    fit_into "$standing" "$out/cutin.png" "$CUTIN_W" "$CUTIN_H"
    echo "    cutin.png (standing art on a wide canvas — drawing one is better)"
  fi

  if [ -f "$src/icon.png" ]; then
    check_png "$src/icon.png" "icon.png"
    fit_into "$src/icon.png" "$out/icon.png" "$ICON" "$ICON"
    echo "    icon.png"
  else
    crop_face "$standing" "$out/icon.png" "$ICON"
    echo "    icon.png (face crop)"
  fi
}

# ---------------------------------------------------------------------------
# ComfyUI
#
# Three calls.  POST /prompt queues a workflow and returns an id, GET
# /history/<id> is empty until it finishes and then holds the outputs, and GET
# /view fetches a file the workflow saved.  Polling is the API's own model:
# there is a websocket for progress, but nothing here needs progress, only
# the result.
# ---------------------------------------------------------------------------

# render_workflow <template> <out> KEY=VALUE...
#
# Substitutes __KEY__ inside the template's *string values*.  Deliberately not
# a text substitution over the raw file: a prompt with a quote or a backslash
# in it would produce malformed JSON, and the failure would land at the far
# end as an unhelpful 400.  Working on the parsed document makes escaping the
# json module's problem instead of ours.
#
# A string that is nothing but a placeholder and resolves to digits becomes a
# number, so a template can carry "seed": "__SEED__" and still send an int.
render_workflow() {
  local template="$1" out="$2"; shift 2
  python3 - "$template" "$out" "$@" <<'PY'
import json, re, sys

template, out, *pairs = sys.argv[1:]
values = dict(p.split('=', 1) for p in pairs)

def sub(node):
    if isinstance(node, dict):
        return {k: sub(v) for k, v in node.items()}
    if isinstance(node, list):
        return [sub(v) for v in node]
    if not isinstance(node, str):
        return node
    whole = re.fullmatch(r'__([A-Z0-9_]+)__', node)
    if whole and whole.group(1) in values:
        v = values[whole.group(1)]
        return int(v) if re.fullmatch(r'-?\d+', v) else v
    return re.sub(r'__([A-Z0-9_]+)__', lambda m: values.get(m.group(1), m.group(0)), node)

with open(template) as f:
    workflow = json.load(f)

left = set()
def scan(node):
    if isinstance(node, dict):
        for v in node.values(): scan(v)
    elif isinstance(node, list):
        for v in node: scan(v)
    elif isinstance(node, str):
        left.update(re.findall(r'__([A-Z0-9_]+)__', node))

workflow = sub(workflow)
scan(workflow)
if left:
    sys.exit(f"workflow template has unfilled placeholders: {', '.join(sorted(left))}")

with open(out, 'w') as f:
    json.dump({"prompt": workflow, "client_id": "bevytris-make-character-art"}, f)
PY
}

# comfy_submit <rendered.json> -> prompt id on stdout
comfy_submit() {
  local body="$1" response
  response="$(curl -sS -f -m 30 -X POST -H 'Content-Type: application/json' \
    --data-binary "@$body" "$COMFYUI_URL/prompt")" || {
    echo "make_character_art.sh: ComfyUI rejected the workflow" >&2
    return 1
  }
  python3 -c "import json,sys; print(json.loads(sys.argv[1])['prompt_id'])" "$response"
}

# comfy_wait <prompt-id> <timeout-seconds>
# Returns once the prompt has an entry in the history.  A queued prompt that
# never runs is the failure this guards against; a slow one is not a failure,
# which is why the timeout is generous and passed in rather than assumed.
comfy_wait() {
  local id="$1" timeout="${2:-600}" i
  for ((i = 0; i < timeout; i++)); do
    if curl -sS -m 10 "$COMFYUI_URL/history/$id" \
      | python3 -c "import json,sys; sys.exit(0 if json.load(sys.stdin) else 1)" 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  echo "make_character_art.sh: ComfyUI did not finish $id within ${timeout}s" >&2
  return 1
}

# comfy_collect <prompt-id> <dest-dir> <name>
# Save the first image the workflow produced as <dest-dir>/<name>.png.
#
# "First" is deliberate: a workflow that saves several images has more than
# one idea about what it is for, and picking between them from out here would
# be guessing.  One workflow, one picture.
comfy_collect() {
  local id="$1" dest="$2" name="$3" spec
  spec="$(curl -sS -f -m 30 "$COMFYUI_URL/history/$id" | python3 - "$id" <<'PY'
import json, sys, urllib.parse
history = json.load(sys.stdin)[sys.argv[1]]
for node in history.get('outputs', {}).values():
    for image in node.get('images', []):
        print(urllib.parse.urlencode({
            'filename': image['filename'],
            'subfolder': image.get('subfolder', ''),
            'type': image.get('type', 'output'),
        }))
        sys.exit(0)
sys.exit("the workflow finished without saving an image")
PY
)" || return 1
  mkdir -p "$dest"
  curl -sS -f -m 60 "$COMFYUI_URL/view?$spec" -o "$dest/$name.png"
}

# generate <workflow-name> <dest-dir> <name> <prompt> <seed>
generate() {
  local workflow="$1" dest="$2" name="$3" prompt="$4" seed="$5"
  local template="$WORKFLOW_DIR/$workflow.json"
  if [ ! -f "$template" ]; then
    echo "make_character_art.sh: no workflow template at $template" >&2
    echo "  Export one from ComfyUI with Save (API Format) and drop it in;" >&2
    echo "  see $WORKFLOW_DIR/README.md for the placeholders it must carry." >&2
    return 1
  fi
  local rendered="$dest/.workflow.$name.json"
  mkdir -p "$dest"
  render_workflow "$template" "$rendered" \
    "PROMPT=$prompt" "SEED=$seed" "WIDTH=$STANDING_W" "HEIGHT=$STANDING_H"
  local id
  id="$(comfy_submit "$rendered")" || return 1
  echo "    queued $name as $id"
  comfy_wait "$id" || return 1
  comfy_collect "$id" "$dest" "$name" || return 1
  /bin/rm -f "$rendered"
  echo "    got $name.png"
}

# ---------------------------------------------------------------------------
# Provenance
#
# CREDITS.md asks that generated art record how it was made, and the only
# version of that which stays true is the one written at the same moment as
# the art.  This lands next to the images rather than in a central file so it
# cannot be separated from what it describes.
# ---------------------------------------------------------------------------
write_source_note() {
  local id="$1" method="$2" detail="$3"
  cat > "$CHARACTERS_DIR/$id/SOURCE.md" <<NOTE
# How this pack's art was made

Generated for bevytris by \`scripts/make_character_art.sh\`.

- Method: $method
$detail

Open-weight image models license their *weights*, not their *outputs*; the
Fair AI Public License and CreativeML OpenRAIL both state that outputs are
not covered and that no contributor claims rights in them. These files are
therefore distributed under this repository's own license.

The voices in \`voices/\` are a separate matter with separate terms — see
\`metadata.json\`'s \`author\` field for the credit they carry.
NOTE
}

# ---------------------------------------------------------------------------
# Modes
# ---------------------------------------------------------------------------
do_check() {
  echo "make_character_art.sh — environment"
  echo
  if [ "$HAVE_SIPS" -eq 1 ]; then echo "  sips      yes"; else echo "  sips      NO (macOS only; needed to derive images)"; fi
  if [ "$HAVE_PYTHON" -eq 1 ]; then echo "  python3   yes"; else echo "  python3   NO (needed for JSON and PNG headers)"; fi
  if comfy_up; then
    echo "  ComfyUI   yes, at $COMFYUI_URL"
  else
    echo "  ComfyUI   NO at $COMFYUI_URL"
    echo "            Set COMFYUI_URL to point at the machine running it."
    echo "            --from <dir> works without it."
  fi
  if [ -d "$WORKFLOW_DIR" ] && find "$WORKFLOW_DIR" -name '*.json' -print -quit | grep -q .; then
    echo "  workflows $(find "$WORKFLOW_DIR" -name '*.json' -exec basename {} .json \; | tr '\n' ' ')"
  else
    echo "  workflows NONE in $WORKFLOW_DIR"
    echo "            Export from ComfyUI with Save (API Format)."
  fi
}

do_from() {
  local src="$1" id="$2"
  if [ "$HAVE_SIPS" -eq 0 ] || [ "$HAVE_PYTHON" -eq 0 ]; then
    echo "make_character_art.sh: skipping — sips and python3 are both needed."
    exit 0
  fi
  [ -d "$src" ] || { echo "make_character_art.sh: $src is not a directory" >&2; exit 1; }
  echo "Building assets/characters/$id/images/ from $src"
  derive_pack "$id" "$src"
  write_source_note "$id" "hand-supplied hero images" \
    "- Source directory: \`$src\` (not part of this repository)
- Derived: standing_left is a mirror of standing_right; anything absent above
  was derived from standing.png by \`scripts/make_character_art.sh\`."
  echo
  echo "Done. Remember metadata.json — scripts/make_dummy_characters.sh writes one."
}

usage() {
  # The header block, which ends where the code starts.
  sed -n '2,/^set -euo pipefail/p' "$self" | grep '^#\|^$' | sed 's|^# \{0,1\}||'
}

main() {
  case "${1:-}" in
    --check) do_check ;;
    --from)
      shift
      [ $# -eq 2 ] || { echo "usage: make_character_art.sh --from <dir> <id>" >&2; exit 2; }
      do_from "$1" "$2"
      ;;
    -h|--help|"") usage ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *)
      echo "make_character_art.sh: generation is not wired up yet." >&2
      echo "  The plumbing is here (see generate/comfy_* in this file) but there" >&2
      echo "  is no workflow template to drive it with, and one cannot be written" >&2
      echo "  without knowing which model and nodes the ComfyUI end has." >&2
      echo "  Run --check to see what is missing, or --from <dir> <id> to derive" >&2
      echo "  a pack from art you already have." >&2
      exit 2
      ;;
  esac
}

main "$@"
