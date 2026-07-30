#!/usr/bin/env bash
#
# make_broken_characters.sh — write HOSTILE character packs for testing.
#
# WHAT THIS IS FOR
#   The game scans assets/characters/ at startup and loads whatever it finds.
#   Anyone can drop a folder in there, so every value the loader reads is
#   somebody else's input, and the loader's contract is that a malformed pack
#   is skipped with a warning naming the folder and the reason — never a
#   panic, never a silent failure, and never a path that escapes the pack.
#
#   A contract of that shape is worth exactly as much as the things you point
#   at it. This writes those things: truncated JSON, a JPEG named .png, a text
#   file named .wav, out-of-range numbers, a display name full of bidi
#   overrides, and a symlink that tries to walk out of the assets directory.
#
#   It used to share a file with a generator for two placeholder characters,
#   `alice` and `bob`. Those are gone — the game ships six real ones now —
#   but the fixtures outlive them, because the loader still has to survive
#   whatever a stranger puts in that folder.
#
# USAGE
#   scripts/make_broken_characters.sh <outdir>
#       <outdir> must be outside the repository; this refuses to write inside
#       it. Fixture descriptions are printed as they are written.
#
# REQUIREMENTS
#   None that are fatal. `sips` makes the fake JPEG a real JPEG rather than a
#   stub, and a VOICEVOX ENGINE makes the misspelled clip a real clip; without
#   either, the fixture degrades into something that still exercises the path.
#
set -euo pipefail

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
  echo "make_broken_characters.sh: $REPO_ROOT does not look like the bevytris repo" >&2
  echo "  (expected Cargo.toml and assets/ next to scripts/)" >&2
  exit 1
fi

HAVE_SIPS=1
command -v sips >/dev/null 2>&1 || HAVE_SIPS=0

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
    echo "make_broken_characters.sh: audio_query failed for $line" >&2
    /bin/rm -f "$query"
    return 1
  fi
  if ! curl -sS -f -m 60 -X POST \
      -H 'Content-Type: application/json' --data-binary "@$query" \
      "$VOICEVOX_URL/synthesis?speaker=$style" -o "$out"; then
    echo "make_broken_characters.sh: synthesis failed for $line" >&2
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
  local credit="${7:-scripts/make_broken_characters.sh}"
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
# Hostile fixtures for negative-path testing
# ===========================================================================
make_broken() {
  local outdir_arg="${1:-}"
  if [ -z "$outdir_arg" ]; then
    echo "make_broken_characters.sh requires an output directory" >&2
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
        echo "make_broken_characters.sh refuses to write inside the repository ($REPO_ROOT)." >&2
        echo "  requested: $path" >&2
        echo "  pick a scratch directory outside the repo instead." >&2
        return 1
        ;;
    esac
    if [ "$path" = "/" ] || [ "$path" = "$HOME" ]; then
      echo "make_broken_characters.sh refuses to write directly into $path" >&2
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
  "author": "scripts/make_broken_characters.sh",
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
  "author": "scripts/make_broken_characters.sh",
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
main() {
  case "${1:-}" in
    -h|--help)
      sed -n '2,/^set -euo pipefail/p' "$0" | grep '^#\|^$' | sed 's|^# \{0,1\}||'
      ;;
    "")
      echo "usage: make_broken_characters.sh <outdir>   (outside the repo)" >&2
      exit 2
      ;;
    *) make_broken "$1" ;;
  esac
}

main "$@"
