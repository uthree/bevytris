#!/usr/bin/env bash
#
# make_dummy_characters.sh — generate throwaway placeholder character packs.
#
# WHAT THIS IS FOR
#   The game loads optional "character" packs from assets/characters/<id>/.
#   A pack is a metadata.json plus standing portraits, an optional cut-in
#   image, up to 17 named voice clips and 20 spoken numbers.  Real
#   characters are drawn and voiced by humans; this script fabricates two
#   obviously-fake stand-ins ("alice", voiced by 四国めたん, and "bob", by
#   ずんだもん) so the loading, layout, font and audio code can be developed
#   and eyeballed without any real art existing yet.
#
#   The art is deliberately ugly and self-describing: every portrait states
#   its own id, its intended facing and its own pixel size, and carries a
#   large block arrow pointing the way the character faces.  If the game ever
#   puts the left-facing art on the left-hand board, or scales a portrait
#   wrongly, the picture itself says so.
#
# THE OUTPUT IS INTENTIONALLY GITIGNORED THROWAWAY DATA.
#   assets/characters/alice/ and assets/characters/bob/ are listed in
#   .gitignore on purpose.  Nothing under them is a source file: re-run this
#   script to recreate them at any time.  Do not commit them, and do not
#   hand-edit them — edit this script instead.
#   assets/characters/README.md *is* committed; it documents the format.
#
#   Note that "do not commit them" is currently a convention, not a legal
#   requirement: the voices below carry terms a game can meet (see WHY
#   VOICEVOX) and the art is our own.  Shipping a pack is a decision about
#   the *art*, not the audio.
#
# USAGE
#   scripts/make_dummy_characters.sh
#       Wipe and regenerate assets/characters/alice and assets/characters/bob
#       relative to the repository root (found from this script's own path,
#       never from $PWD).
#
#   scripts/make_dummy_characters.sh --broken <outdir>
#       Write HOSTILE fixtures for negative-path testing into <outdir>.
#       <outdir> must be outside the repository; this mode refuses to write
#       inside it.  See the fixture descriptions printed at run time.
#
# REQUIREMENTS
#   macOS `sips`, to rasterise the placeholder SVGs at an exact pixel size.
#   VOICEVOX ENGINE, for the voices — see WHY VOICEVOX below.
#   On a machine without them (e.g. Linux CI) the script prints a notice and
#   exits 0 rather than failing the build.
#
# WHY VOICEVOX
#   The voices used to come from macOS `say`.  That was convenient and
#   completely unshippable: Apple's system voices are licensed for use *on*
#   a Mac, not for baking into a game someone else downloads, so every clip
#   the script produced was one that could never leave this machine.
#
#   VOICEVOX states its terms plainly instead.  Audio generated from a voice
#   library may be used commercially and non-commercially as long as the
#   character is credited — "VOICEVOX:ずんだもん" and so on — which is a
#   condition a game can actually meet.  The engine is a local HTTP server;
#   nothing is uploaded and no account is involved.  It also runs on Linux
#   and Windows, so this script is no longer macOS-only for the audio half.
#
#     https://voicevox.hiroshiba.jp/term/        the software terms
#     https://zunko.jp/con_ongen_kiyaku.html     terms for the voices used here
#
#   Each pack records the credit line it owes in its own metadata.json, so
#   the obligation travels with the files instead of living only in here.
#
#   NOTE: the *art* is still procedural placeholder SVG, deliberately.  The
#   illustrations these characters are usually drawn with are third-party
#   assets whose terms permit use in a finished work but not redistribution
#   of the files themselves, which is exactly what committing them to a
#   public repository would be.  Voices are shippable; art is not yet.
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Locate ourselves.  Everything is resolved from the script path so that the
# script behaves identically no matter what $PWD is.
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

# Sanity: refuse to run from a tree that does not look like this repository,
# so a stray copy of the script cannot scribble into an unrelated directory.
if [ ! -f "$REPO_ROOT/Cargo.toml" ] || [ ! -d "$REPO_ROOT/assets" ]; then
  echo "make_dummy_characters.sh: $REPO_ROOT does not look like the bevytris repo" >&2
  echo "  (expected Cargo.toml and assets/ next to scripts/)" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Tool detection.  Missing tools are a soft skip, not a failure.
# ---------------------------------------------------------------------------
HAVE_SIPS=1
command -v sips >/dev/null 2>&1 || HAVE_SIPS=0

# VOICEVOX ENGINE speaks over HTTP.  $VOICEVOX_URL points this at an engine
# that is already running — a Docker container, a remote box, a different
# port — and is the whole configuration surface.
VOICEVOX_URL="${VOICEVOX_URL:-http://127.0.0.1:50021}"

# The engine bundled inside the desktop app, which is the one most people
# have.  Only consulted when nothing is answering already.
VOICEVOX_BUNDLED='/Applications/VOICEVOX.app/Contents/Resources/vv-engine/run'

# PID of an engine *this script* started, so it is also this script's job to
# stop it.  An engine that was already up is left exactly as it was found.
VOICEVOX_PID=''

engine_up() {
  curl -sS -m 3 "$VOICEVOX_URL/version" >/dev/null 2>&1
}

stop_engine() {
  if [ -n "$VOICEVOX_PID" ]; then
    kill "$VOICEVOX_PID" 2>/dev/null || true
    wait "$VOICEVOX_PID" 2>/dev/null || true
    VOICEVOX_PID=''
  fi
}
trap stop_engine EXIT

# Return 0 with an engine reachable at $VOICEVOX_URL, 1 with none available.
start_engine() {
  if engine_up; then
    echo "VOICEVOX ENGINE: already running at $VOICEVOX_URL"
    return 0
  fi
  if [ ! -x "$VOICEVOX_BUNDLED" ]; then
    return 1
  fi
  echo "VOICEVOX ENGINE: starting the one bundled with VOICEVOX.app"
  "$VOICEVOX_BUNDLED" --host 127.0.0.1 --port 50021 >/dev/null 2>&1 &
  VOICEVOX_PID=$!
  # First start loads the models, which is not instant.
  local i
  for i in $(seq 1 60); do
    if engine_up; then
      echo "VOICEVOX ENGINE: up after ${i}s"
      return 0
    fi
    # A crashed engine will never answer, so stop waiting for it.
    if ! kill -0 "$VOICEVOX_PID" 2>/dev/null; then
      VOICEVOX_PID=''
      return 1
    fi
    sleep 1
  done
  stop_engine
  return 1
}

# ---------------------------------------------------------------------------
# Palette / typography
# ---------------------------------------------------------------------------
# Concrete bold faces.  sips ignores a font-weight attribute entirely, and it
# also ignores PostScript-style face names such as "Helvetica-Bold" or
# "Arial-BoldMT" (those silently fall back to the regular weight).  Only
# families whose *family* name carries the weight actually come out bold, so
# these two names are load-bearing — do not "tidy" them.
FONT_ASCII="Arial Black"
FONT_JA="Hiragino Sans W7"      # the Japanese bold face sips will actually use

STANDING_W=512
STANDING_H=1024
CUTIN_W=1024
CUTIN_H=512
ICON_W=256
ICON_H=256

# The seventeen named voice kinds, with the English and Japanese line for
# each.  "kind|english|japanese" — bash 3.2 has no associative arrays.
VOICE_LINES='
select|Leave it to me|まかせて
ready|Here we go|いくよー
clear|Nice|いいね
tetris|Tetris!|テトリス！
tspin|T spin!|ティースピン！
perfect_clear|Perfect clear!|パーフェクトクリア！
combo|Combo!|コンボ！
attack|Take this|くらえー
counter|Not today!|そうはさせない！
damage|Ouch|いたっ
damage_heavy|That is a lot|うわ、多い！
pinch|This is bad|まずい、まずい
zone_ready|Zone, ready|ゾーン、いけるよ
zone_start|Zone, activate|ゾーン、発動
zone_finish|How was that|どうだった
win|I win!|わたしの勝ち！
lose|No way|そんなー
'

# The spoken numbers, for counting a combo or a zone's banked lines.
# "n|english|japanese", and the file is count_NN.wav (zero-padded).
COUNT_LINES='
1|one|いち
2|two|に
3|three|さん
4|four|よん
5|five|ご
6|six|ろく
7|seven|なな
8|eight|はち
9|nine|きゅう
10|ten|じゅう
11|eleven|じゅういち
12|twelve|じゅうに
13|thirteen|じゅうさん
14|fourteen|じゅうよん
15|fifteen|じゅうご
16|sixteen|じゅうろく
17|seventeen|じゅうなな
18|eighteen|じゅうはち
19|nineteen|じゅうきゅう
20|twenty|にじゅう
'

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# render_svg <svg-file> <png-file> <expected-w> <expected-h>
# Rasterise an SVG and assert that the result really is that many pixels.
render_svg() {
  local svg="$1" png="$2" want_w="$3" want_h="$4"
  sips -s format png "$svg" --out "$png" >/dev/null 2>&1
  local got_w got_h
  got_w="$(sips -g pixelWidth "$png" 2>/dev/null | awk '/pixelWidth/ {print $2}')"
  got_h="$(sips -g pixelHeight "$png" 2>/dev/null | awk '/pixelHeight/ {print $2}')"
  if [ "$got_w" != "$want_w" ] || [ "$got_h" != "$want_h" ]; then
    echo "make_dummy_characters.sh: $png is ${got_w}x${got_h}, expected ${want_w}x${want_h}" >&2
    exit 1
  fi
}

# standing_svg <facing:right|left> <id> <name1> <name2> <name-font>
#              <body> <accent> <head> <text> <muted>
# Emits an SVG for a 512x1024 standing portrait on stdout.  The canvas is
# transparent outside the body so the alpha path gets exercised too.
standing_svg() {
  local facing="$1" id="$2" name1="$3" name2="$4" name_font="$5"
  local body="$6" accent="$7" head="$8" text="$9" muted="${10}"

  local facing_word arrow stripe_x eye1 eye2 nose
  if [ "$facing" = "right" ]; then
    facing_word="RIGHT"
    # Block arrow pointing right, centred on x=256.
    arrow="126,580 286,580 286,530 386,620 286,710 286,660 126,660"
    stripe_x=440           # accent stripe marks the FRONT of the character
    eye1=286; eye2=324; nose=350
  else
    facing_word="LEFT"
    arrow="386,580 226,580 226,530 126,620 226,710 226,660 386,660"
    stripe_x=32
    eye1=226; eye2=188; nose=162
  fi

  cat <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="$STANDING_W" height="$STANDING_H" viewBox="0 0 $STANDING_W $STANDING_H">
  <rect x="16" y="16" width="480" height="992" rx="28" fill="$body" stroke="$accent" stroke-width="8"/>
  <rect x="$stripe_x" y="24" width="40" height="976" rx="16" fill="$accent" opacity="0.55"/>
  <circle cx="256" cy="200" r="110" fill="$head" stroke="$accent" stroke-width="6"/>
  <circle cx="$eye1" cy="178" r="16" fill="$text"/>
  <circle cx="$eye2" cy="178" r="16" fill="$text"/>
  <circle cx="$nose" cy="228" r="9" fill="$accent"/>
  <text x="256" y="420" font-family="$FONT_ASCII" font-size="44" fill="$muted" text-anchor="middle">id: $id</text>
  <polygon points="$arrow" fill="$accent"/>
  <text x="256" y="800" font-family="$name_font" font-size="68" fill="$text" text-anchor="middle">$name1</text>
  <text x="256" y="872" font-family="$name_font" font-size="68" fill="$text" text-anchor="middle">$name2</text>
  <text x="256" y="940" font-family="$FONT_ASCII" font-size="54" fill="$accent" text-anchor="middle">$facing_word</text>
  <text x="256" y="985" font-family="$FONT_ASCII" font-size="28" fill="$muted" text-anchor="middle">${STANDING_W}x${STANDING_H}</text>
</svg>
SVG
}

# cutin_svg <display-name> <name-font> <body> <accent> <text> <muted>
cutin_svg() {
  local name="$1" name_font="$2" body="$3" accent="$4" text="$5" muted="$6"
  cat <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="$CUTIN_W" height="$CUTIN_H" viewBox="0 0 $CUTIN_W $CUTIN_H">
  <rect x="8" y="8" width="1008" height="496" rx="20" fill="$body" stroke="$accent" stroke-width="6"/>
  <polygon points="60,504 180,8 300,8 180,504" fill="$accent" opacity="0.35"/>
  <polygon points="740,504 860,8 980,8 860,504" fill="$accent" opacity="0.35"/>
  <text x="512" y="230" font-family="$name_font" font-size="96" fill="$text" text-anchor="middle">$name</text>
  <text x="512" y="345" font-family="$FONT_ASCII" font-size="80" fill="$accent" text-anchor="middle">CUT-IN</text>
  <text x="512" y="425" font-family="$FONT_ASCII" font-size="36" fill="$muted" text-anchor="middle">${CUTIN_W}x${CUTIN_H}</text>
</svg>
SVG
}

# icon_svg <name1> <name-font> <body> <accent> <head> <text>
# Square art for the character picker tile.
icon_svg() {
  local name1="$1" name_font="$2" body="$3" accent="$4" head="$5" text="$6"
  cat <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="$ICON_W" height="$ICON_H" viewBox="0 0 $ICON_W $ICON_H">
  <rect x="6" y="6" width="244" height="244" rx="24" fill="$body" stroke="$accent" stroke-width="6"/>
  <circle cx="128" cy="104" r="62" fill="$head" stroke="$accent" stroke-width="5"/>
  <circle cx="106" cy="94" r="9" fill="$text"/>
  <circle cx="150" cy="94" r="9" fill="$text"/>
  <text x="128" y="212" font-family="$name_font" font-size="44" fill="$text" text-anchor="middle">$name1</text>
</svg>
SVG
}

# result_svg <won:win|lose> <name1> <name-font> <body> <accent> <head> <text> <muted>
# The pose held while the result is on screen. Deliberately a different
# shape from the standing art so a swap that does not happen is obvious.
result_svg() {
  local outcome="$1" name1="$2" name_font="$3" body="$4" accent="$5" head="$6" text="$7" muted="$8"
  local word mouth arms
  if [ "$outcome" = "win" ]; then
    word="WIN"
    mouth='<path d="M 206 250 Q 256 300 306 250" fill="none" stroke="'"$text"'" stroke-width="12"/>'
    # Both arms up.
    arms='<polygon points="120,430 176,380 216,470 176,520"  fill="'"$accent"'"/>
    <polygon points="392,430 336,380 296,470 336,520" fill="'"$accent"'"/>'
  else
    word="LOSE"
    mouth='<path d="M 206 290 Q 256 240 306 290" fill="none" stroke="'"$text"'" stroke-width="12"/>'
    # Both arms down.
    arms='<polygon points="120,560 176,610 216,520 176,470" fill="'"$accent"'"/>
    <polygon points="392,560 336,610 296,520 336,470" fill="'"$accent"'"/>'
  fi
  cat <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="$STANDING_W" height="$STANDING_H" viewBox="0 0 $STANDING_W $STANDING_H">
  <rect x="16" y="16" width="480" height="992" rx="28" fill="$body" stroke="$accent" stroke-width="10"/>
  <circle cx="256" cy="220" r="120" fill="$head" stroke="$accent" stroke-width="6"/>
  <circle cx="216" cy="190" r="17" fill="$text"/>
  <circle cx="296" cy="190" r="17" fill="$text"/>
  $mouth
  $arms
  <text x="256" y="720" font-family="$FONT_ASCII" font-size="120" fill="$accent" text-anchor="middle">$word</text>
  <text x="256" y="830" font-family="$name_font" font-size="60" fill="$text" text-anchor="middle">$name1</text>
  <text x="256" y="900" font-family="$FONT_ASCII" font-size="34" fill="$muted" text-anchor="middle">result pose</text>
</svg>
SVG
}

# speak <style-id> <text> <outfile>
#
# Two calls, which is how the engine is designed: /audio_query turns the text
# into an editable prosody document, /synthesis renders that document.  The
# document is passed straight through unedited — pitch and speed are the
# character's own, and second-guessing them is how a voice pack starts
# sounding like a robot reading a phrasebook.
#
# The engine returns mono 16-bit PCM in a RIFF/WAVE container at 24 kHz,
# which is exactly what Bevy's wav decoder wants, so there is nothing to
# convert.  `-G` puts the parameters in the query string while `-X POST`
# keeps the method the engine expects; this is pure curl on purpose, so the
# script gains no new dependency.
speak() {
  local style="$1" line="$2" out="$3"
  local query="$out.query.json"
  if ! curl -sS -f -m 30 -G \
      --data-urlencode "text=$line" \
      --data-urlencode "speaker=$style" \
      -X POST "$VOICEVOX_URL/audio_query" -o "$query"; then
    echo "make_dummy_characters.sh: audio_query failed for $line" >&2
    /bin/rm -f "$query"
    return 1
  fi
  if ! curl -sS -f -m 60 -X POST \
      -H 'Content-Type: application/json' --data-binary "@$query" \
      "$VOICEVOX_URL/synthesis?speaker=$style" -o "$out"; then
    echo "make_dummy_characters.sh: synthesis failed for $line" >&2
    /bin/rm -f "$query" "$out"
    return 1
  fi
  /bin/rm -f "$query"
}

# write_metadata <dir> <display_name> <ascii_name> <flavor> <alpha> <gain> [credit]
# The folder name is the id; there is deliberately no id field.
#
# `author` doubles as the credit line the pack owes.  VOICEVOX requires the
# character to be named wherever the audio is used, and the picker already
# shows this field under the focused character — so a pack that speaks with
# a borrowed voice says whose it is, on screen, without anybody having to
# remember to add it somewhere else.
write_metadata() {
  local dir="$1" display_name="$2" ascii_name="$3" flavor="$4" alpha="$5" gain="$6"
  local credit="${7:-scripts/make_dummy_characters.sh}"
  cat > "$dir/metadata.json" <<JSON
{
  "schema": 1,
  "display_name": "$display_name",
  "ascii_name": "$ascii_name",
  "flavor": "$flavor",
  "author": "$credit",
  "portrait_alpha": $alpha,
  "voice_gain": $gain
}
JSON
}

# summarise <label> <dir...> — count regular files and total bytes.
summarise() {
  local label="$1"; shift
  local n bytes
  set +e
  read -r n bytes < <(find "$@" -type f -exec wc -c {} \; 2>/dev/null \
    | awk '{s += $1; n += 1} END {printf "%d %d\n", n + 0, s + 0}')
  set -e
  echo "$label: $n files, $bytes bytes"
}

# ===========================================================================
# --broken: hostile fixtures for negative-path testing
# ===========================================================================
make_broken() {
  local outdir_arg="${1:-}"
  if [ -z "$outdir_arg" ]; then
    echo "--broken requires an output directory" >&2
    exit 2
  fi

  # Refuse to write anywhere inside the repository: these fixtures are
  # designed to break loaders and must never be mistaken for real assets.
  # Checked once textually *before* creating anything, so a rejected path does
  # not leave a stray directory behind, and once more after canonicalisation
  # so that symlinks and ".." segments cannot sneak past the first check.
  refuse_if_in_repo() {
    local path="$1"
    case "$path/" in
      "$REPO_ROOT"/*)
        echo "--broken refuses to write inside the repository ($REPO_ROOT)." >&2
        echo "  requested: $path" >&2
        echo "  pick a scratch directory outside the repo instead." >&2
        return 1
        ;;
    esac
    if [ "$path" = "/" ] || [ "$path" = "$HOME" ]; then
      echo "--broken refuses to write directly into $path" >&2
      return 1
    fi
    return 0
  }

  local outdir
  case "$outdir_arg" in
    /*) outdir="$outdir_arg" ;;
    *) outdir="$(pwd -P)/$outdir_arg" ;;
  esac
  refuse_if_in_repo "$outdir" || exit 2

  local pre_existing=1
  [ -d "$outdir" ] || pre_existing=0
  mkdir -p "$outdir"
  outdir="$(cd "$outdir" && pwd -P)"
  if ! refuse_if_in_repo "$outdir"; then
    # Undo our own mkdir so a rejected run is a complete no-op.
    [ "$pre_existing" -eq 0 ] && rmdir "$outdir" 2>/dev/null
    exit 2
  fi

  echo "Writing hostile character fixtures into:"
  echo "  $outdir"
  echo

  # Idempotency: remove only the fixtures this script owns.
  local owned='bad_json no_images nasty_display_name jpeg_as_png text_as_wav
    misspelled_voice .hidden_leading_dot out_of_range traversal'
  local f
  for f in $owned; do
    /bin/rm -rf "$outdir/$f"
  done
  /bin/rm -rf "$outdir/has space" "$outdir/OUTSIDE_TARGET.txt"

  local good_meta_display='DUMMY FIXTURE'

  # ---- 1. malformed JSON -------------------------------------------------
  mkdir -p "$outdir/bad_json/images"
  cat > "$outdir/bad_json/metadata.json" <<'JSON'
{
  "schema": 1,
  "display_name": "TRAILING COMMA AND NO CLOSING BRACE",
  "ascii_name": "BAD JSON",
  "portrait_alpha": 0.2,
JSON
  echo "  bad_json/                  metadata.json is truncated with a trailing comma."
  echo "                             HOSTILE: parser must report a readable error and skip"
  echo "                             the character, not panic or abort startup."

  # ---- 2. valid JSON, no images -----------------------------------------
  mkdir -p "$outdir/no_images"
  write_metadata "$outdir/no_images" "$good_meta_display" "NO IMAGES" \
    "valid metadata, zero art" "0.2" "1.0"
  echo "  no_images/                 valid metadata.json, no images/ directory at all."
  echo "                             HOSTILE: every image is optional per the contract, so"
  echo "                             this must load and render with no portrait, not crash"
  echo "                             on a missing asset handle."

  # ---- 3. nasty display_name --------------------------------------------
  mkdir -p "$outdir/nasty_display_name"
  # 200 decoded characters containing a right-to-left override (U+202E), a
  # zero-width space (U+200B) and a newline.  Written as JSON \u escapes so
  # the fixture file itself stays pure ASCII and unambiguous in a diff.
  local pad RLO ZWSP
  pad="$(printf 'A%.0s' $(seq 1 190))"
  RLO='\u202e'    # right-to-left override, as a JSON escape
  ZWSP='\u200b'   # zero-width space, as a JSON escape
  cat > "$outdir/nasty_display_name/metadata.json" <<JSON
{
  "schema": 1,
  "display_name": "${RLO}EVIL${ZWSP}\nBOB$pad",
  "ascii_name": "NASTY NAME",
  "flavor": "display_name is 200 chars with RLO, ZWSP and a newline",
  "author": "scripts/make_dummy_characters.sh --broken",
  "portrait_alpha": 0.2,
  "voice_gain": 1.0
}
JSON
  mkdir -p "$outdir/nasty_display_name/images"
  echo "  nasty_display_name/        display_name decodes to 200 chars including U+202E"
  echo "                             (right-to-left override), U+200B (zero-width space)"
  echo "                             and a literal newline."
  echo "                             HOSTILE: must be length-clamped and stripped of control"
  echo "                             and bidi characters before it reaches a single-line UI"
  echo "                             label; a raw newline must not break the layout and the"
  echo "                             RLO must not reverse surrounding menu text."

  # ---- 4. JPEG renamed to .png -----------------------------------------
  mkdir -p "$outdir/jpeg_as_png/images"
  write_metadata "$outdir/jpeg_as_png" "$good_meta_display" "JPEG AS PNG" \
    "standing_right.png is really a JPEG" "0.2" "1.0"
  if [ "$HAVE_SIPS" -eq 1 ]; then
    local tmp_svg="$outdir/jpeg_as_png/.tmp.svg"
    cat > "$tmp_svg" <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
  <rect width="64" height="64" fill="#ff00ff"/>
</svg>
SVG
    sips -s format jpeg "$tmp_svg" --out "$outdir/jpeg_as_png/images/standing_right.png" >/dev/null 2>&1
    /bin/rm -f "$tmp_svg"
    echo "  jpeg_as_png/               images/standing_right.png contains real JPEG bytes."
  else
    printf '\377\330\377\340NOT-A-PNG' > "$outdir/jpeg_as_png/images/standing_right.png"
    echo "  jpeg_as_png/               images/standing_right.png contains JPEG magic bytes"
    echo "                             (sips unavailable, so it is a stub, not a full JPEG)."
  fi
  echo "                             HOSTILE: the loader must not trust the extension. Either"
  echo "                             sniff the content or handle the decode error per-asset."

  # ---- 5. text file renamed to .wav ------------------------------------
  mkdir -p "$outdir/text_as_wav/voices"
  write_metadata "$outdir/text_as_wav" "$good_meta_display" "TEXT AS WAV" \
    "voices/tetris.wav is plain text" "0.2" "1.0"
  printf 'this is not audio, it is a text file pretending to be one\n' \
    > "$outdir/text_as_wav/voices/tetris.wav"
  echo "  text_as_wav/               voices/tetris.wav is UTF-8 text, no RIFF header."
  echo "                             HOSTILE: a failed audio decode must disable that one clip,"
  echo "                             not kill the audio system or the whole character."

  # ---- 6. misspelled voice file ----------------------------------------
  mkdir -p "$outdir/misspelled_voice/voices"
  write_metadata "$outdir/misspelled_voice" "$good_meta_display" "MISSPELLED" \
    "has tetriss.wav, not tetris.wav" "0.2" "1.0"
  # The fixture only needs a *valid* clip under an invalid name, so a stub
  # is a fine substitute when there is no engine to ask.
  if engine_up || start_engine; then
    speak 3 "テトリス！" "$outdir/misspelled_voice/voices/tetriss.wav"
  else
    printf 'RIFF----WAVE (stub: no VOICEVOX ENGINE available)' \
      > "$outdir/misspelled_voice/voices/tetriss.wav"
  fi
  echo "  misspelled_voice/          a valid clip at voices/tetriss.wav; tetris.wav is absent."
  echo "                             HOSTILE: unknown filenames must be ignored silently-ish"
  echo "                             (ideally warned about once), and the missing tetris.wav"
  echo "                             must simply mean 'no line for that event'."

  # ---- 7. illegal folder names ----------------------------------------
  mkdir -p "$outdir/.hidden_leading_dot/images"
  write_metadata "$outdir/.hidden_leading_dot" "$good_meta_display" "DOTFILE DIR" \
    "folder name starts with a dot" "0.2" "1.0"
  mkdir -p "$outdir/has space/images"
  write_metadata "$outdir/has space" "$good_meta_display" "SPACE IN DIR" \
    "folder name contains a space" "0.2" "1.0"
  echo "  .hidden_leading_dot/       folder name begins with '.'."
  echo "  'has space'/               folder name contains a space."
  echo "                             HOSTILE: the folder name IS the character id, so it also"
  echo "                             ends up in save files, config and log lines. Dot-prefixed"
  echo "                             directories should be skipped as hidden, and ids should"
  echo "                             be restricted to a documented charset rather than"
  echo "                             accepting whatever the filesystem allows."

  # ---- 8. out-of-range numbers ----------------------------------------
  mkdir -p "$outdir/out_of_range/images"
  write_metadata "$outdir/out_of_range" "$good_meta_display" "OUT OF RANGE" \
    "portrait_alpha 99, voice_gain -5" "99" "-5"
  echo "  out_of_range/              portrait_alpha = 99, voice_gain = -5."
  echo "                             HOSTILE: must clamp to 0.05..0.45 and 0.2..2.0. An"
  echo "                             unclamped alpha hides the board; a negative gain"
  echo "                             inverts or blows up the mixer."

  # ---- 9. parent-directory traversal ----------------------------------
  printf 'A file OUTSIDE any character directory. Reading this means traversal succeeded.\n' \
    > "$outdir/OUTSIDE_TARGET.txt"
  mkdir -p "$outdir/traversal/images" "$outdir/traversal/voices"
  cat > "$outdir/traversal/metadata.json" <<'JSON'
{
  "schema": 1,
  "display_name": "../../../../etc/hosts",
  "ascii_name": "TRAVERSAL",
  "flavor": "images/standing_right.png is a symlink pointing out of the pack",
  "author": "scripts/make_dummy_characters.sh --broken",
  "portrait_alpha": 0.2,
  "voice_gain": 1.0
}
JSON
  # A filename that escapes the pack via a relative symlink.
  ln -s "../../OUTSIDE_TARGET.txt" "$outdir/traversal/images/standing_left.png"
  # A literal filename carrying an encoded traversal, in case anything
  # url-decodes or unescapes a path component before joining it.
  printf 'encoded traversal in the filename\n' \
    > "$outdir/traversal/images/..%2F..%2FOUTSIDE_TARGET.png"
  # A directory entry made only of dots, which some path normalisers mishandle.
  mkdir -p "$outdir/traversal/images/...."
  echo "  traversal/                 images/standing_left.png is a relative symlink to"
  echo "                             ../../OUTSIDE_TARGET.txt; images/..%2F..%2F...png is a"
  echo "                             literal filename holding an encoded '../../'; there is"
  echo "                             also a '....' directory. display_name is a path too."
  echo "                             HOSTILE: asset paths must be confined to the pack. Do not"
  echo "                             follow symlinks out of assets/, do not unescape path"
  echo "                             components, and never treat metadata strings as paths."

  echo
  summarise "Hostile fixtures" "$outdir"
  echo "Note: 'find -type f' does not count the symlink; the pack contains one."
  echo "None of these fixtures live inside the repository."
}

# ===========================================================================
# Normal mode: the two dummy characters
# ===========================================================================
make_character() {
  local id="$1" display_name="$2" ascii_name="$3" flavor="$4" \
        style="$5" credit="$6" name1="$7" name2="$8" name_font="$9" \
        body="${10}" accent="${11}" head="${12}" text="${13}" muted="${14}" \
        alpha="${15}" gain="${16}"

  local dir="$CHARACTERS_DIR/$id"
  /bin/rm -rf "$dir"
  mkdir -p "$dir/images" "$dir/voices"

  write_metadata "$dir" "$display_name" "$ascii_name" "$flavor" "$alpha" "$gain" "$credit"

  local tmp_svg="$dir/.render.svg"
  local facing
  for facing in right left; do
    standing_svg "$facing" "$id" "$name1" "$name2" "$name_font" \
      "$body" "$accent" "$head" "$text" "$muted" > "$tmp_svg"
    render_svg "$tmp_svg" "$dir/images/standing_$facing.png" "$STANDING_W" "$STANDING_H"
  done
  cutin_svg "$display_name" "$name_font" "$body" "$accent" "$text" "$muted" > "$tmp_svg"
  render_svg "$tmp_svg" "$dir/images/cutin.png" "$CUTIN_W" "$CUTIN_H"
  icon_svg "$name1" "$name_font" "$body" "$accent" "$head" "$text" > "$tmp_svg"
  render_svg "$tmp_svg" "$dir/images/icon.png" "$ICON_W" "$ICON_H"
  local outcome
  for outcome in win lose; do
    result_svg "$outcome" "$name1" "$name_font" \
      "$body" "$accent" "$head" "$text" "$muted" > "$tmp_svg"
    render_svg "$tmp_svg" "$dir/images/$outcome.png" "$STANDING_W" "$STANDING_H"
  done
  /bin/rm -f "$tmp_svg"

  # Every line is spoken in Japanese: VOICEVOX's libraries are Japanese
  # voices, and handing one an English sentence gets it read out as
  # katakana. The English column stays in the tables above because it is
  # what documents what each clip is *for*.
  local kind en ja
  echo "$VOICE_LINES" | while IFS='|' read -r kind en ja; do
    [ -n "$kind" ] || continue
    speak "$style" "$ja" "$dir/voices/$kind.wav"
  done
  echo "$COUNT_LINES" | while IFS='|' read -r kind en ja; do
    [ -n "$kind" ] || continue
    speak "$style" "$ja" "$(printf '%s/voices/count_%02d.wav' "$dir" "$kind")"
  done

  local named counts
  named="$(echo "$VOICE_LINES" | grep -c '|')"
  counts="$(echo "$COUNT_LINES" | grep -c '|')"
  echo "  $id: metadata.json, 6 images, $named voices + $counts numbers ($credit)"
}

make_all() {
  local missing=0
  if [ "$HAVE_SIPS" -eq 0 ]; then
    echo "make_dummy_characters.sh: missing sips (SVG -> PNG), which is macOS-only."
    missing=1
  fi
  if ! start_engine; then
    echo "make_dummy_characters.sh: no VOICEVOX ENGINE at $VOICEVOX_URL."
    echo "  Open VOICEVOX.app, or run the engine yourself and set VOICEVOX_URL:"
    echo "    docker run --rm -p 50021:50021 voicevox/voicevox_engine:cpu-latest"
    missing=1
  fi
  if [ "$missing" -eq 1 ]; then
    echo "  The dummy pack is throwaway test data; skipping it is not an error."
    exit 0
  fi

  mkdir -p "$CHARACTERS_DIR"
  echo "Generating dummy character packs under:"
  echo "  $CHARACTERS_DIR"
  echo

  # Two voices that sound nothing like each other, so a clip playing on the
  # wrong board is audible rather than merely wrong.  The style numbers are
  # VOICEVOX style ids; `curl $VOICEVOX_URL/speakers` lists them all.
  #
  # alice — 四国めたん ノーマル, teal.  ASCII display name.
  make_character alice \
    "DUMMY ALICE" "DUMMY ALICE" \
    "テスト用のダミーキャラ。右を向いている。" \
    2 "VOICEVOX:四国めたん" \
    "DUMMY" "ALICE" "$FONT_ASCII" \
    "#123f3a" "#39e6b8" "#1c5d55" "#f2fffb" "#9fe8d6" \
    "0.20" "1.0"

  # bob — ずんだもん ノーマル, magenta.  Non-ASCII display name on purpose:
  # this is what exercises the game's Japanese pixel-font path and the
  # ascii_name fallback.
  make_character bob \
    "ダミー・ボブ" "DUMMY BOB" \
    "テスト用のダミーキャラ。左を向いている。" \
    3 "VOICEVOX:ずんだもん" \
    "ダミー" "ボブ" "$FONT_JA" \
    "#3a1240" "#ff5cc8" "#571a5e" "#fff2fb" "#f0a8dd" \
    "0.20" "1.0"

  echo
  echo "Files written:"
  find "$CHARACTERS_DIR/alice" "$CHARACTERS_DIR/bob" -type f \
    | sed "s|^$REPO_ROOT/||" | sort
  echo
  summarise "Total" "$CHARACTERS_DIR/alice" "$CHARACTERS_DIR/bob"
  echo "This output is gitignored throwaway data — re-run this script to recreate it."
}

# ===========================================================================
main() {
  case "${1:-}" in
    --broken) shift; make_broken "${1:-}" ;;
    "") make_all ;;
    -h|--help)
      sed -n '2,40p' "$self" | sed 's|^# \{0,1\}||'
      ;;
    *)
      echo "unknown argument: $1" >&2
      echo "usage: make_dummy_characters.sh [--broken <outdir>]" >&2
      exit 2
      ;;
  esac
}

main "$@"
