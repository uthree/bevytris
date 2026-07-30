#!/usr/bin/env bash
#
# make_characters.sh — build the six character packs that ship with the game.
#
# WHAT THIS IS
#   The cast. Six original characters, each a folder under
#   assets/characters/ containing a metadata.json, portrait art and a voice.
#   Unlike the throwaway fixtures from make_dummy_characters.sh, these are
#   committed and distributed, so everything here has to be something we are
#   actually allowed to hand to somebody else.
#
# WHAT IS OURS AND WHAT IS BORROWED
#   The *designs* are original. That was a deliberate turn: adopting an
#   existing character would have meant working under that character's own
#   terms — for the obvious candidates, non-commercial only and a licence
#   contract otherwise — and pinning the whole repository to it. Original
#   characters carry no such condition, and open-weight image models place
#   their conditions on the weights rather than the outputs, so the art
#   carries this repository's own licence.
#
#   The *voices* are borrowed and credited. Audio generated from a VOICEVOX
#   voice library may be used commercially and non-commercially provided the
#   character is named wherever the audio is used, so each pack writes its
#   credit into metadata.json's `author`, which the picker shows on screen.
#   See assets/CREDITS.md.
#
# DESIGN RULES, AND WHY
#   Every one of these exists because a generator asked for detail returns
#   noise, and a portrait behind a playfield has to read at a glance:
#
#     * one dominant hue, white, and one dark neutral. One accent, no more.
#     * hues spaced around the wheel, so any two boards are distinguishable.
#       The sixth is achromatic on purpose: something has to be legible next
#       to all five of the others.
#     * the silhouette identifies, not the colour. A stack of blocks covers
#       most of a portrait; hair shape is what survives.
#     * one signature accessory each, and it has to be a thing a prompt can
#       name. The first draft gave one character a hood with eyes on it,
#       which was charming and unrepeatable — a design the prompt cannot
#       describe again is a design that breaks the moment you need it in a
#       different pose.
#
# USAGE
#   scripts/make_characters.sh              everything, every character
#   scripts/make_characters.sh mint rosa    just those
#   scripts/make_characters.sh --voices     voices and metadata only
#   scripts/make_characters.sh --art        art only
#
# REQUIREMENTS
#   A VOICEVOX ENGINE for the voices (started automatically if VOICEVOX.app
#   is installed; $VOICEVOX_URL to point elsewhere), a ComfyUI for the art
#   ($COMFYUI_URL), and macOS `sips` to fit the images. Missing pieces are
#   reported and skipped rather than failed.
#
set -euo pipefail

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

if [ ! -f "$REPO_ROOT/Cargo.toml" ] || [ ! -d "$REPO_ROOT/assets" ]; then
  echo "make_characters.sh: $REPO_ROOT does not look like the bevytris repo" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# The cast
#
# id|ascii|display|flavor|voice-name|style-id|design
#
# `style-id` is a VOICEVOX *style*, not a character — one character usually
# has several. `curl $VOICEVOX_URL/speakers` lists them all.
# ---------------------------------------------------------------------------
CAST='
mint|MINT|ミント|落ち着いていて、いつも音楽を聴いている。|四国めたん|2|teal hair, short bob cut, blunt bangs, aqua eyes, white headphones, white long sleeve shirt, teal skirt, teal and white two tone color scheme
rosa|ROSA|ローザ|よく喋る。長い髪が自慢。|春日部つむぎ|8|magenta hair, very long low twintails, pink eyes, single red hair ribbon, white blouse, magenta vest, magenta skirt, magenta and white two tone color scheme
amber|AMBER|アンバー|元気。とにかく速く積む。|ずんだもん|3|orange hair, high side ponytail, amber eyes, goggles on forehead, white tee shirt, orange overalls, orange and white two tone color scheme
vio|VIO|ヴィオ|物静か。長考派。|冥鳴ひまり|14|purple hair, long straight hair, violet eyes, plain hooded cape, hood down, white dress, plain purple cape, purple and white two tone color scheme
cobalt|COBALT|コバルト|マイペース。マフラーは一年中。|九州そら|16|dark blue hair, short messy hair, blue eyes, long white scarf, navy blue hoodie, white shirt, navy blue and white two tone color scheme
ash|ASH|アッシュ|無口。表情は読めない。|東北きりたん|108|white hair, short bob cut, hair clip, grey eyes, grey hoodie, white skirt, white and grey clothes, monochrome color scheme
'

# ---------------------------------------------------------------------------
# Prompts
# ---------------------------------------------------------------------------
STYLE="masterpiece, best quality, high score, great score, anime style, game character, 1girl, solo, young girl, 5 heads tall, full body, facing viewer, thick black outline, bold lineart, cel shading, flat color, simple design, limited color palette, large expressive eyes, eye highlights, detailed hands, five fingers, simple background, white background"

# All three poses stand, and all three keep the face up and visible.
#
# Both of those were learned the hard way. The board anchors a portrait at
# the bottom of the field, so a figure caught mid-air reads as one with no
# feet — the first pass at a victory pose produced six characters jumping
# with their legs tucked under them. And "head down" for a loss lands
# squarely on the model's horror-girl attractor: face swallowed by hair,
# arms dangling. A loss should read as disappointed, not haunted.
pose_prompt() {
  case "$1" in
    standing) echo "standing, arms at sides, open hands, looking at viewer, cheerful smile" ;;
    win)      echo "standing, both arms raised high, victory, open mouth smile, closed eyes, happy, feet on the ground" ;;
    lose)     echo "standing, sad expression, teary eyes, frowning, hands clasped together, looking at viewer, disappointed, feet on the ground" ;;
    *) echo "make_characters.sh: unknown pose $1" >&2; return 1 ;;
  esac
}

NEG="lowres, bad anatomy, bad hands, mitten hands, blob hands, malformed hands, fused fingers, missing fingers, extra digits, text, error, cropped, worst quality, low quality, low score, bad score, signature, watermark, blurry, realistic, 3d, photo, gradient, soft shading, thin lines, multiple views, sketch, border, frame, chibi, super deformed, minimalist, vector art, blank eyes, cluttered, multiple girls, 2girls, crowd, wings, scenery, floating objects, floating orb, sphere, ball, tall, long legs, mature female, adult, rainbow, multicolored, colorful, many colors, ornate, intricate, busy pattern, frills, jewelry, tiara, armor, mecha, horror, scary, creepy, ghost, face covered by hair, hair over face, faceless, duplicate head, extra head, puddle, splash, ground shadow, colored floor, jumping, mid-air, flying, falling, kneeling, sitting, crouching, lying down, from above, animal hood, creature hood, eyes on hood, kigurumi, black dress, grey background, colored background, gradient background"

# One seed for the whole cast. Same seed and same design across the three
# poses is what keeps a character recognisably one person: with only the
# pose tokens changing, the sampler lands somewhere adjacent rather than on
# somebody else.
SEED="${SEED:-31415}"

# ---------------------------------------------------------------------------
# Voice lines. The same table make_dummy_characters.sh uses, because it is
# the game's list of voice kinds and there is only one of those.
# ---------------------------------------------------------------------------
VOICE_LINES='
select|まかせて
ready|いくよー
clear|いいね
tetris|テトリス！
tspin|ティースピン！
perfect_clear|パーフェクトクリア！
combo|コンボ！
attack|くらえー
counter|そうはさせない！
damage|いたっ
damage_heavy|うわ、多い！
pinch|まずい、まずい
zone_ready|ゾーン、いけるよ
zone_start|ゾーン、発動
zone_finish|どうだった
win|わたしの勝ち！
lose|そんなー
'

COUNT_LINES='
1|いち
2|に
3|さん
4|よん
5|ご
6|ろく
7|なな
8|はち
9|きゅう
10|じゅう
11|じゅういち
12|じゅうに
13|じゅうさん
14|じゅうよん
15|じゅうご
16|じゅうろく
17|じゅうなな
18|じゅうはち
19|じゅうきゅう
20|にじゅう
'

# ---------------------------------------------------------------------------
# Engines
# ---------------------------------------------------------------------------
VOICEVOX_URL="${VOICEVOX_URL:-http://127.0.0.1:50021}"
VOICEVOX_BUNDLED='/Applications/VOICEVOX.app/Contents/Resources/vv-engine/run'
VOICEVOX_PID=''
COMFYUI_URL="${COMFYUI_URL:-http://127.0.0.1:8188}"

stop_engine() {
  if [ -n "$VOICEVOX_PID" ]; then
    kill "$VOICEVOX_PID" 2>/dev/null || true
    wait "$VOICEVOX_PID" 2>/dev/null || true
    VOICEVOX_PID=''
  fi
}
trap stop_engine EXIT

voicevox_up() { curl -sS -m 3 "$VOICEVOX_URL/version" >/dev/null 2>&1; }
comfyui_up() { curl -sS -m 5 "$COMFYUI_URL/system_stats" >/dev/null 2>&1; }

start_voicevox() {
  voicevox_up && return 0
  [ -x "$VOICEVOX_BUNDLED" ] || return 1
  echo "VOICEVOX ENGINE: starting the one bundled with VOICEVOX.app"
  "$VOICEVOX_BUNDLED" --host 127.0.0.1 --port 50021 >/dev/null 2>&1 &
  VOICEVOX_PID=$!
  local i
  for i in $(seq 1 60); do
    voicevox_up && return 0
    kill -0 "$VOICEVOX_PID" 2>/dev/null || { VOICEVOX_PID=''; return 1; }
    sleep 1
  done
  stop_engine
  return 1
}

# speak <style-id> <text> <outfile>
speak() {
  local style="$1" line="$2" out="$3" query="$3.query.json"
  curl -sS -f -m 30 -G --data-urlencode "text=$line" --data-urlencode "speaker=$style" \
    -X POST "$VOICEVOX_URL/audio_query" -o "$query"
  curl -sS -f -m 60 -X POST -H 'Content-Type: application/json' --data-binary "@$query" \
    "$VOICEVOX_URL/synthesis?speaker=$style" -o "$out"
  /bin/rm -f "$query"
}

# ---------------------------------------------------------------------------
# Building one character
# ---------------------------------------------------------------------------
build_voices() {
  local id="$1" style="$2" voice_name="$3" display="$4" ascii="$5" flavor="$6"
  local dir="$CHARACTERS_DIR/$id"
  mkdir -p "$dir/voices"

  cat > "$dir/metadata.json" <<JSON
{
  "schema": 1,
  "display_name": "$display",
  "ascii_name": "$ascii",
  "flavor": "$flavor",
  "author": "VOICEVOX:$voice_name",
  "portrait_alpha": 0.20,
  "voice_gain": 1.0
}
JSON

  local kind text
  echo "$VOICE_LINES" | while IFS='|' read -r kind text; do
    [ -n "$kind" ] || continue
    speak "$style" "$text" "$dir/voices/$kind.wav"
  done
  echo "$COUNT_LINES" | while IFS='|' read -r kind text; do
    [ -n "$kind" ] || continue
    speak "$style" "$text" "$(printf '%s/voices/count_%02d.wav' "$dir" "$kind")"
  done
  echo "    metadata.json + $(find "$dir/voices" -name '*.wav' | wc -l | tr -d ' ') voices ($voice_name)"
}

build_art() {
  local id="$1" design="$2"
  local hero
  hero="$(mktemp -d)"
  local pose
  for pose in standing win lose; do
    "$SCRIPT_DIR/make_character_art.sh" --generate \
      "$STYLE, $(pose_prompt "$pose"), $design" "$NEG" "$SEED" "$hero/$pose.png" || {
        /bin/rm -rf "$hero"; return 1;
      }
  done
  "$SCRIPT_DIR/make_character_art.sh" --from "$hero" "$id"
  /bin/rm -rf "$hero"
}

# ---------------------------------------------------------------------------
main() {
  local do_voices=1 do_art=1
  case "${1:-}" in
    --voices) do_art=0; shift ;;
    --art) do_voices=0; shift ;;
    -h|--help) sed -n '2,/^set -euo pipefail/p' "$self" | grep '^#\|^$' | sed 's|^# \{0,1\}||'; exit 0 ;;
  esac
  local want="$*"

  if [ "$do_voices" -eq 1 ] && ! start_voicevox; then
    echo "make_characters.sh: no VOICEVOX ENGINE at $VOICEVOX_URL — skipping voices."
    echo "  Open VOICEVOX.app, or: docker run --rm -p 50021:50021 voicevox/voicevox_engine:cpu-latest"
    do_voices=0
  fi
  if [ "$do_art" -eq 1 ] && ! comfyui_up; then
    echo "make_characters.sh: no ComfyUI at $COMFYUI_URL — skipping art."
    echo "  Set COMFYUI_URL to the machine running it."
    do_art=0
  fi
  if [ "$do_voices" -eq 0 ] && [ "$do_art" -eq 0 ]; then
    echo "  Nothing to do."
    exit 0
  fi

  local id ascii display flavor voice_name style design
  echo "$CAST" | while IFS='|' read -r id ascii display flavor voice_name style design; do
    [ -n "$id" ] || continue
    if [ -n "$want" ] && ! echo " $want " | grep -q " $id "; then continue; fi
    echo "--- $id ($ascii)"
    [ "$do_art" -eq 1 ] && build_art "$id" "$design"
    [ "$do_voices" -eq 1 ] && build_voices "$id" "$style" "$voice_name" "$display" "$ascii" "$flavor"
  done
  echo
  echo "Committed assets. Their licence is not the code's — see assets/CREDITS.md."
}

main "$@"
