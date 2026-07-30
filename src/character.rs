//! Characters: a face on the board, a voice for what just happened, and a
//! cut-in for the moments worth interrupting the screen for.
//!
//! The roster is not compiled in. Every character is a folder under
//! `assets/characters/`, scanned once at startup:
//!
//! ```text
//! assets/characters/<id>/metadata.json
//!                       /images/standing_right.png   faces right -> left-hand board
//!                       /images/standing_left.png    faces left  -> right-hand board
//!                       /images/cutin.png            optional, wide
//!                       /voices/tetris.wav           optional, one per VoiceKind
//! ```
//!
//! The folder name *is* the id, which deletes the whole duplicate-id
//! problem. Everything else lives in `metadata.json`.
//!
//! That layout is a feature, not an implementation detail: anyone can drop
//! a folder in and it turns up in the picker. So every value read here is
//! treated as hostile input. A malformed character is skipped with a
//! `warn!` naming the folder and the reason; nothing in this module may
//! panic on a file it did not write, and nothing may go silent without
//! saying why. That is also why the parsing half is pure functions with
//! tests and the `std::fs` half is a thin shim over them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::asset::io::file::FileAssetReader;
use bevy::audio::Volume;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use serde::Deserialize;

use crate::config::GameSettings;
use crate::i18n::Locale;
use crate::core::game::{ClearKind, GameEvent, LineClear};
use crate::emissive;
use crate::render::{BoardTheme, CELL_Z};
use crate::session::{BoardEvent, BoardIndex};
use crate::state::{AppState, GameMode, PlayState};

// ---------------------------------------------------------------------------
// What a character says
// ---------------------------------------------------------------------------

/// One line of voice acting. The file under `voices/` is named after the
/// variant, lowercased — `VoiceKind::PerfectClear` is `perfect_clear.wav`.
///
/// The list is deliberately short. Every entry has to map to something the
/// game already reports and to a moment a player would notice; a character
/// that comments on every lock is a character you turn off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum VoiceKind {
    /// Confirmed in the picker. The only line a player hears before the
    /// match, and the one that sells the character they just chose.
    Select,
    Ready,
    Clear,
    Tetris,
    TSpin,
    PerfectClear,
    Combo,
    Attack,
    /// An attack that wiped out every row of incoming garbage on its way
    /// past. Not a bigger attack — a save.
    Counter,
    /// A couple of rows of garbage: a complaint.
    Damage,
    /// [`HEAVY_GARBAGE_ROWS`] or more at once: a different noise entirely.
    DamageHeavy,
    /// The stack got close enough to the ceiling to be a problem.
    Pinch,
    /// The zone gauge just filled, so there is now something to spend.
    ZoneReady,
    ZoneStart,
    ZoneFinish,
    Win,
    Lose,
}

impl VoiceKind {
    pub const ALL: [VoiceKind; 17] = [
        VoiceKind::Select,
        VoiceKind::Ready,
        VoiceKind::Clear,
        VoiceKind::Tetris,
        VoiceKind::TSpin,
        VoiceKind::PerfectClear,
        VoiceKind::Combo,
        VoiceKind::Attack,
        VoiceKind::Counter,
        VoiceKind::Damage,
        VoiceKind::DamageHeavy,
        VoiceKind::Pinch,
        VoiceKind::ZoneReady,
        VoiceKind::ZoneStart,
        VoiceKind::ZoneFinish,
        VoiceKind::Win,
        VoiceKind::Lose,
    ];

    pub fn file_stem(self) -> &'static str {
        match self {
            VoiceKind::Select => "select",
            VoiceKind::Ready => "ready",
            VoiceKind::Clear => "clear",
            VoiceKind::Tetris => "tetris",
            VoiceKind::TSpin => "tspin",
            VoiceKind::PerfectClear => "perfect_clear",
            VoiceKind::Combo => "combo",
            VoiceKind::Attack => "attack",
            VoiceKind::Counter => "counter",
            VoiceKind::Damage => "damage",
            VoiceKind::DamageHeavy => "damage_heavy",
            VoiceKind::Pinch => "pinch",
            VoiceKind::ZoneReady => "zone_ready",
            VoiceKind::ZoneStart => "zone_start",
            VoiceKind::ZoneFinish => "zone_finish",
            VoiceKind::Win => "win",
            VoiceKind::Lose => "lose",
        }
    }

    /// How loudly this line insists. A bigger number cuts a smaller one
    /// off mid-word; an equal or smaller one waits its turn (which, for a
    /// voice, means it is dropped — a queued reaction to something that
    /// happened four seconds ago is worse than silence).
    pub fn priority(self) -> u8 {
        match self {
            VoiceKind::Clear => 1,
            VoiceKind::Combo => 2,
            VoiceKind::Attack => 2,
            VoiceKind::Tetris | VoiceKind::TSpin | VoiceKind::Damage => 3,
            VoiceKind::ZoneReady => 3,
            VoiceKind::Ready
            | VoiceKind::ZoneStart
            | VoiceKind::ZoneFinish
            | VoiceKind::DamageHeavy
            | VoiceKind::Counter
            | VoiceKind::Pinch => 4,
            VoiceKind::PerfectClear => 5,
            // Nothing else is happening at either of these moments.
            VoiceKind::Win | VoiceKind::Lose | VoiceKind::Select => 6,
        }
    }

    /// The line to use when the pack does not have this one.
    ///
    /// These are not niceties. Every pack authored before a variant
    /// existed has none of its file, and going *silent* at a moment the
    /// character used to react to would make the pack strictly worse than
    /// it was yesterday. So each new line borrows the nearest old one:
    /// less specific, still a reaction.
    ///
    /// [`VoiceKind::Pinch`] and [`VoiceKind::ZoneReady`] deliberately have
    /// none. They mark states rather than events, and an old line
    /// borrowed for them would fire at a moment it does not describe —
    /// yelping "ouch" at a tall stack nobody hit you with.
    pub fn fallback(self) -> Option<VoiceKind> {
        match self {
            VoiceKind::DamageHeavy => Some(VoiceKind::Damage),
            VoiceKind::Counter => Some(VoiceKind::Attack),
            VoiceKind::Select => Some(VoiceKind::Ready),
            _ => None,
        }
    }
}

/// One thing a character can be asked to say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceLine {
    /// A named reaction.
    Kind(VoiceKind),
    /// A number, spoken: the n-th clear of a combo, or the n-th line a
    /// zone has banked.
    ///
    /// `instead` is what to say for a pack that ships no numbers, so
    /// counting is an upgrade to a pack rather than a replacement for it.
    Count { n: u32, instead: Option<VoiceKind> },
}

impl VoiceLine {
    fn priority(self) -> u8 {
        match self {
            VoiceLine::Kind(kind) => kind.priority(),
            // Chatter-level, which is what makes counting work: each
            // number cuts the one before it (equal priority replaces), and
            // anything that actually matters — a tetris, a hit — cuts the
            // count.
            VoiceLine::Count { .. } => 2,
        }
    }
}

/// How high a character counts. Twenty covers every combo and every zone
/// bank a real match produces; past that the numbers run out and the
/// moment speaks for itself.
pub const COUNT_MAX: u32 = 20;

/// The file a spoken number lives in. Zero-padded so a pack's `voices/`
/// folder sorts the way it reads.
fn count_stem(n: u32) -> String {
    format!("count_{n:02}")
}

/// Rows of garbage that make a hit worth its own voice line. Four is a
/// tetris' worth arriving at once — the point where the stack you were
/// building stops being the stack you have.
const HEAVY_GARBAGE_ROWS: u32 = 4;

/// Rows a counter has to absorb to be worth remarking on, and to be worth
/// stopping the screen for. Deliberately the same shape as the damage
/// thresholds above: cancelling two rows is the mirror of taking two, and
/// cancelling a tetris' worth is the mirror of eating one.
const COUNTER_ROWS: u32 = 2;
const COUNTER_CUTIN_ROWS: u32 = HEAVY_GARBAGE_ROWS;

/// The moves worth stopping the screen for, in the order they outrank each
/// other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CutInKind {
    Tetris,
    TSpinDouble,
    TSpinTriple,
    /// A tetris' worth of incoming garbage cancelled outright.
    Counter,
    ZoneRelease,
    PerfectClear,
}

impl CutInKind {
    fn priority(self) -> u8 {
        match self {
            CutInKind::Tetris => 1,
            CutInKind::TSpinDouble => 2,
            CutInKind::TSpinTriple => 3,
            CutInKind::Counter => 4,
            CutInKind::ZoneRelease => 5,
            CutInKind::PerfectClear => 6,
        }
    }

    /// The colour the art is washed with, so a glance says which move it
    /// was. The move's *name* is not drawn here — the clear banners over
    /// the board already announce it, and this is scenery now.
    fn tint(self) -> Color {
        match self {
            CutInKind::Tetris => Color::srgb(0.35, 0.85, 1.0),
            CutInKind::TSpinDouble | CutInKind::TSpinTriple => Color::srgb(0.85, 0.45, 1.0),
            // Warm, and nothing else here is: a counter is the one cut-in
            // earned by what the *opponent* did.
            CutInKind::Counter => Color::srgb(1.0, 0.5, 0.25),
            CutInKind::ZoneRelease => Color::srgb(1.0, 0.82, 0.35),
            CutInKind::PerfectClear => Color::srgb(1.0, 0.95, 0.6),
        }
    }
}

/// How many lines a zone release has to clear before it earns a cut-in.
const ZONE_CUTIN_LINES: u32 = 5;

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// `metadata.json`, exactly as it appears on disk.
///
/// Every field is optional but `display_name`: a folder that only wants a
/// name should need only a name. Unknown keys are ignored rather than
/// rejected, so a pack authored against a later version of this game still
/// loads for the parts we understand.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct CharacterMeta {
    schema: Option<u32>,
    display_name: Option<String>,
    ascii_name: Option<String>,
    flavor: Option<String>,
    author: Option<String>,
    portrait_alpha: Option<f32>,
    voice_gain: Option<f32>,
}

/// The schema this build was written against. A pack declaring a newer one
/// still loads — it is warned about, not rejected, because "your character
/// vanished" is a worse outcome than "your character is missing whatever
/// the new field did".
const SCHEMA: u32 = 1;

/// A character the game can actually use. Paths are asset-relative
/// (`characters/alice/voices/tetris.wav`), which is what `AssetServer`
/// wants and is deliberately not an absolute path — nothing downstream
/// should be able to reach outside the assets root.
#[derive(Clone, Debug)]
pub struct Character {
    pub id: String,
    /// Shown in the picker and on the cut-in. May be any script.
    pub display_name: String,
    /// The same name, guaranteed ASCII, for anywhere the 8x8 pixel font
    /// would otherwise draw tofu.
    pub ascii_name: String,
    pub flavor: String,
    pub author: String,
    /// Facing right, then facing left.
    pub standing: [Option<String>; 2],
    pub cutin: Option<String>,
    /// The pose held while the result is on screen: winning, then losing.
    /// Optional — a pack with neither keeps standing still.
    pub result: [Option<String>; 2],
    /// Square-ish art for the picker tile.
    pub icon: Option<String>,
    /// Indexed by [`VoiceKind`] discriminant.
    pub voices: Vec<Option<String>>,
    /// The spoken numbers, `counts[n - 1]` for "n". Separate from
    /// `voices` because it is a range rather than a list: adding a
    /// nineteenth `VoiceKind` for the word "nineteen" would say nothing
    /// about the game and everything about the enum.
    pub counts: Vec<Option<String>>,
    pub portrait_alpha: f32,
    pub voice_gain: f32,
    /// Problems found while loading this character, for the picker to
    /// flag. A character with issues is still playable.
    pub issues: Vec<String>,
}

impl Character {
    pub fn voice(&self, kind: VoiceKind) -> Option<&str> {
        self.clip(kind)
            .or_else(|| kind.fallback().and_then(|to| self.clip(to)))
    }

    /// The clip for a line, and whether it is a *counted* one.
    ///
    /// The caller needs both: a spoken number is one of a rapid sequence
    /// and must not be rationed, while the named line it falls back to is
    /// ordinary chatter and must be. Resolving the fallback is the only
    /// place that difference is knowable.
    fn line_clip(&self, line: VoiceLine) -> Option<(&str, bool)> {
        match line {
            VoiceLine::Kind(kind) => self.voice(kind).map(|path| (path, false)),
            VoiceLine::Count { n, instead } => match self.count(n) {
                Some(path) => Some((path, true)),
                None => instead
                    .and_then(|kind| self.voice(kind))
                    .map(|path| (path, false)),
            },
        }
    }

    fn count(&self, n: u32) -> Option<&str> {
        self.counts
            .get(n.checked_sub(1)? as usize)
            .and_then(|v| v.as_deref())
    }

    fn clip(&self, kind: VoiceKind) -> Option<&str> {
        self.voices
            .get(kind as usize)
            .and_then(|v| v.as_deref())
    }

    /// The winning or losing pose, falling back to however they were
    /// already standing.
    pub fn result_art(&self, slot: usize, won: bool) -> Option<&str> {
        let want = usize::from(!won);
        self.result[want]
            .as_deref()
            .or(self.result[1 - want].as_deref())
            .or_else(|| self.standing_for(slot))
    }

    /// The art for a character standing on board `slot`: the left-hand
    /// board looks right, the right-hand board looks left, so both face
    /// the middle. A pack that shipped only one facing gets it reused
    /// rather than nothing at all.
    pub fn standing_for(&self, slot: usize) -> Option<&str> {
        let want = if slot == 1 { 1 } else { 0 };
        self.standing[want]
            .as_deref()
            .or(self.standing[1 - want].as_deref())
    }
}

/// Everything installed, in a stable order. Scanned once at startup;
/// nothing reloads it, so an index into this is a valid handle for the
/// lifetime of the process.
#[derive(Resource, Default)]
pub struct CharacterRoster(pub Vec<Character>);

impl CharacterRoster {
    pub fn get(&self, index: usize) -> Option<&Character> {
        self.0.get(index)
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.0.iter().position(|c| c.id == id)
    }
}

/// Everything the scan complained about, so the player learns about a
/// broken folder without reading a terminal.
#[derive(Resource, Default)]
pub struct CharacterIssues(pub Vec<String>);

/// Handles for the two characters this match needs. Loading is by asset
/// path and cached, so re-resolving every match is free.
#[derive(Resource, Default)]
pub struct CharacterAssets {
    images: HashMap<String, Handle<Image>>,
    audio: HashMap<String, Handle<AudioSource>>,
}

impl CharacterAssets {
    fn image(&mut self, server: &AssetServer, path: &str) -> Handle<Image> {
        self.images
            .entry(path.to_string())
            .or_insert_with(|| server.load(path.to_string()))
            .clone()
    }

    fn sound(&mut self, server: &AssetServer, path: &str) -> Handle<AudioSource> {
        self.audio
            .entry(path.to_string())
            .or_insert_with(|| server.load(path.to_string()))
            .clone()
    }
}

/// Who each side is playing as this match, as roster indices. Index
/// matches `BoardIndex`.
///
/// Written by the picker before the match starts and *not* cleared by
/// `spawn_session`, which is the whole point: a restart keeps the
/// characters the player already chose.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct MatchCharacters {
    pub sides: [Option<usize>; 2],
}

// ---------------------------------------------------------------------------
// Parsing — pure, hostile-input-facing, tested
// ---------------------------------------------------------------------------

/// A folder name we are willing to treat as an id. Kept to what is safe in
/// a log line, a settings file and a path: no separators, no leading dot,
/// nothing invisible.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Strip everything that has no business in a single-line label, then cap
/// the length by *characters*. Bidi overrides and zero-width joiners are
/// removed rather than escaped: this text is drawn in a pixel font, and
/// the only thing they can do here is scramble the line around it.
fn sanitize_text(raw: &str, max_chars: usize) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            let n = *c as u32;
            // C0/C1 controls, zero-width and bidi-override blocks, BOM.
            !(n < 0x20
                || (0x7F..=0x9F).contains(&n)
                || (0x200B..=0x200F).contains(&n)
                || (0x202A..=0x202E).contains(&n)
                || (0x2060..=0x206F).contains(&n)
                || n == 0xFEFF)
        })
        .map(|c| if c == '\t' { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max_chars).collect()
}

fn ascii_only(raw: &str, max_chars: usize) -> String {
    raw.chars()
        .filter(|c| (' '..='~').contains(c))
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A readable name from a folder name, for a pack that gave no ASCII one.
fn name_from_id(id: &str) -> String {
    id.replace(['_', '-'], " ").to_uppercase()
}

fn clamp_or(value: Option<f32>, default: f32, lo: f32, hi: f32) -> f32 {
    match value {
        Some(v) if v.is_finite() => v.clamp(lo, hi),
        _ => default,
    }
}

const PORTRAIT_ALPHA_DEFAULT: f32 = 0.20;

/// Turn parsed metadata into a character. Paths are filled in by the
/// scanner afterwards; everything here is text handling, which is where
/// the sharp edges are.
fn finalize(id: &str, meta: CharacterMeta, issues: &mut Vec<String>) -> Character {
    if let Some(schema) = meta.schema
        && schema > SCHEMA
    {
        issues.push(format!(
            "{id}: metadata.json declares schema {schema}, this build knows {SCHEMA} — loading what it understands"
        ));
    }
    let ascii_raw = meta
        .ascii_name
        .as_deref()
        .map(|a| ascii_only(a, 24))
        .filter(|a| !a.is_empty());
    let display = meta
        .display_name
        .as_deref()
        .map(|d| sanitize_text(d, 24))
        .filter(|d| !d.is_empty());
    let ascii_name = ascii_raw
        .clone()
        .or_else(|| display.as_deref().map(|d| ascii_only(d, 24)))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| name_from_id(id));
    let display_name = display.unwrap_or_else(|| ascii_name.clone());
    Character {
        id: id.to_string(),
        display_name,
        ascii_name,
        flavor: meta.flavor.as_deref().map(|f| sanitize_text(f, 80)).unwrap_or_default(),
        author: meta.author.as_deref().map(|a| sanitize_text(a, 32)).unwrap_or_default(),
        standing: [None, None],
        cutin: None,
        result: [None, None],
        icon: None,
        voices: vec![None; VoiceKind::ALL.len()],
        counts: vec![None; COUNT_MAX as usize],
        portrait_alpha: clamp_or(meta.portrait_alpha, PORTRAIT_ALPHA_DEFAULT, 0.05, 0.45),
        voice_gain: clamp_or(meta.voice_gain, 1.0, 0.2, 2.0),
        issues: Vec::new(),
    }
}

/// Parse a `metadata.json`.
///
/// The object check is not pedantry: serde will happily fill a struct from a
/// JSON *array* positionally, so `["Alice"]` would quietly load as a
/// character and `[]` as a nameless one. A pack that malformed deserves to
/// be told, not guessed at.
fn parse_metadata(text: &str) -> Result<CharacterMeta, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !value.is_object() {
        return Err("the top level must be a JSON object".to_string());
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Width and height out of a PNG's IHDR, or `None` if this is not a PNG at
/// all. Sniffing the header is the cheapest way to catch the commonest
/// authoring mistake by a mile: a JPEG (or a whole different file) saved
/// under a `.png` name.
fn png_dimensions(head: &[u8]) -> Option<(u32, u32)> {
    const MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if head.len() < 24 || head[..8] != MAGIC || &head[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    Some((w, h))
}

/// Is this a RIFF/WAVE file? Bevy is built with the `wav` feature only, so
/// anything else under `voices/` would decode to silence with no clue why.
fn is_riff_wave(head: &[u8]) -> bool {
    head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE"
}

/// The WAVE format tags this build can actually play: uncompressed PCM and
/// IEEE float. Anything else — mu-law, ADPCM, an mp3 in a RIFF wrapper —
/// is a format the decoder will refuse.
const WAVE_PCM: u16 = 1;
const WAVE_IEEE_FLOAT: u16 = 3;
const WAVE_EXTENSIBLE: u16 = 0xFFFE;

/// Walk a RIFF file's chunk list and check that it is really playable.
///
/// A twelve-byte `RIFF....WAVE` sniff is not enough, and the shortfall is
/// not cosmetic: bevy hands the bytes to `rodio::Decoder::new(..).unwrap()`
/// at *playback* time, so a truncated download or a mu-law clip under
/// `voices/` takes the whole game down mid-match. Since a pack is someone
/// else's file, the only acceptable place to find that out is here.
///
/// Chunk payloads are skipped rather than read, which matters: `say` writes
/// a four-kilobyte `FLLR` pad between `fmt ` and `data`, so the interesting
/// headers are nowhere near the front of the file.
fn wav_ok(path: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};

    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_AUDIO_BYTES {
        return Err(format!(
            "{size} bytes is over the {MAX_AUDIO_BYTES} limit"
        ));
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut head = [0u8; 12];
    file.read_exact(&mut head)
        .map_err(|_| "the file is too short to be a WAV".to_string())?;
    if !is_riff_wave(&head) {
        return Err("not a RIFF/WAVE file (this build decodes .wav only)".to_string());
    }
    let mut format: Option<u16> = None;
    let mut have_data = false;
    let mut at = 12u64;
    // A malformed size field could otherwise walk this forever.
    for _ in 0..64 {
        if at + 8 > size {
            break;
        }
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        let mut header = [0u8; 8];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as u64;
        match &header[..4] {
            b"fmt " => {
                let mut fmt = [0u8; 2];
                file.read_exact(&mut fmt).map_err(|e| e.to_string())?;
                format = Some(u16::from_le_bytes(fmt));
            }
            b"data" => have_data = true,
            _ => {}
        }
        // Chunks are word-aligned, and an odd length is padded.
        at += 8 + len + (len % 2);
    }
    match format {
        None => Err("the WAV has no fmt chunk".to_string()),
        Some(tag)
            if tag != WAVE_PCM && tag != WAVE_IEEE_FLOAT && tag != WAVE_EXTENSIBLE =>
        {
            Err(format!(
                "WAVE format {tag} is not uncompressed PCM; re-export as 16-bit PCM"
            ))
        }
        Some(_) if !have_data => Err("the WAV has no data chunk".to_string()),
        Some(_) => Ok(()),
    }
}

/// Levenshtein distance, capped — only used to turn `tetriss.wav` into a
/// "did you mean" line, so it never needs to be fast.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Every file name a `voices/` folder may hold, named lines and spoken
/// numbers alike.
fn voice_file_stems() -> Vec<String> {
    VoiceKind::ALL
        .iter()
        .map(|k| k.file_stem().to_string())
        .chain((1..=COUNT_MAX).map(count_stem))
        .collect()
}

/// The voice file this name was probably meant to be. `count_3.wav` for
/// `count_03.wav` is the single likeliest slip a pack can make, which is
/// why the numbers are in the candidate list too.
fn nearest_voice_name(stem: &str) -> Option<String> {
    voice_file_stems()
        .into_iter()
        .filter(|name| edit_distance(stem, name) <= 2)
        .min_by_key(|name| edit_distance(stem, name))
}

/// Which voice a game event asks for, and which cut-in (if any) it earns.
///
/// Split out from the systems so the mapping itself is testable: this is
/// the table that decides what a character is *for*, and getting it wrong
/// is silent.
fn voice_for(event: &GameEvent, two_boards: bool) -> Option<VoiceLine> {
    let named = |kind| Some(VoiceLine::Kind(kind));
    match event {
        GameEvent::Cleared(c) => {
            if c.perfect_clear {
                named(VoiceKind::PerfectClear)
            } else if matches!(c.kind, ClearKind::TSpin | ClearKind::TSpinMini) {
                named(VoiceKind::TSpin)
            } else if c.lines >= 4 {
                named(VoiceKind::Tetris)
            } else if c.combo >= 1 {
                // `combo` counts from zero, so this is the second clear of
                // a streak: "two". A pack with no numbers says its combo
                // line instead, which is what it did before counting
                // existed.
                Some(VoiceLine::Count {
                    n: c.combo + 1,
                    instead: Some(VoiceKind::Combo),
                })
            } else if two_boards && c.attack > 0 {
                named(VoiceKind::Attack)
            } else {
                named(VoiceKind::Clear)
            }
        }
        GameEvent::TSpinNoLines { .. } => named(VoiceKind::TSpin),
        GameEvent::GarbageRose { rows } if *rows >= HEAVY_GARBAGE_ROWS => {
            named(VoiceKind::DamageHeavy)
        }
        GameEvent::GarbageRose { rows } if *rows >= 2 => named(VoiceKind::Damage),
        GameEvent::Countered { rows } if *rows >= COUNTER_ROWS => named(VoiceKind::Counter),
        GameEvent::ZoneReady => named(VoiceKind::ZoneReady),
        GameEvent::ZoneActivated => named(VoiceKind::ZoneStart),
        // Counting the bank up as it grows. There is no named line to fall
        // back on: a pack with no numbers stays quiet here and speaks at
        // the release, exactly as it did before.
        GameEvent::ZoneLines { total, .. } => Some(VoiceLine::Count {
            n: *total,
            instead: None,
        }),
        GameEvent::ZoneEnded { lines, .. } if *lines > 0 => named(VoiceKind::ZoneFinish),
        _ => None,
    }
}

fn cutin_for(event: &GameEvent) -> Option<CutInKind> {
    match event {
        GameEvent::Cleared(c) => Some(cutin_for_clear(c)?),
        GameEvent::Countered { rows } if *rows >= COUNTER_CUTIN_ROWS => Some(CutInKind::Counter),
        GameEvent::ZoneEnded { lines, .. } if *lines >= ZONE_CUTIN_LINES => {
            Some(CutInKind::ZoneRelease)
        }
        _ => None,
    }
}

fn cutin_for_clear(c: &LineClear) -> Option<CutInKind> {
    if c.perfect_clear {
        return Some(CutInKind::PerfectClear);
    }
    if matches!(c.kind, ClearKind::TSpin) {
        return match c.lines {
            3 => Some(CutInKind::TSpinTriple),
            2 => Some(CutInKind::TSpinDouble),
            _ => None,
        };
    }
    (c.lines >= 4).then_some(CutInKind::Tetris)
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// Where the character folders live. `FileAssetReader`'s base path is what
/// `AssetServer` itself resolves against — the crate root under `cargo
/// run`, the executable's directory in a shipped build — so this cannot
/// drift away from where the handles are loaded from.
fn characters_dir() -> PathBuf {
    FileAssetReader::get_base_path()
        .join("assets")
        .join("characters")
}

/// Caps. A character pack is art, not a payload: anything past these is a
/// mistake or an attack, and either way we are better off saying so than
/// loading it.
const MAX_META_BYTES: u64 = 64 * 1024;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 4 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 4096;
const MAX_ROSTER: usize = 32;

/// Read `head_len` bytes off the front of a file, refusing anything over
/// `max_bytes` before it is opened.
fn peek(path: &Path, max_bytes: u64, head_len: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > max_bytes {
        return Err(format!("{size} bytes is over the {max_bytes} limit"));
    }
    if size == 0 {
        return Err("the file is empty".to_string());
    }
    let mut buf = vec![0u8; head_len.min(size as usize)];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .read_exact(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Scan `assets/characters/`. Returns the roster and every complaint, in
/// that order. A missing directory is not a complaint — a build that ships
/// no characters is a legitimate build.
fn scan_characters() -> (Vec<Character>, Vec<String>) {
    let dir = characters_dir();
    let mut issues: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (Vec::new(), issues);
    };
    let mut found: Vec<Character> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let raw = entry.file_name();
        let Some(id) = raw.to_str() else {
            issues.push(format!("{:?}: the folder name is not valid UTF-8", raw));
            continue;
        };
        // Hidden folders are the operating system's business, not ours.
        if id.starts_with('.') {
            continue;
        }
        if !valid_id(id) {
            issues.push(format!(
                "{id}: folder names may only use letters, digits, '_' and '-' (max 48)"
            ));
            continue;
        }
        if names.iter().any(|n| n.eq_ignore_ascii_case(id)) {
            issues.push(format!("{id}: another folder already claims that name"));
            continue;
        }
        match load_character(&entry.path(), id, &mut issues) {
            Some(character) => {
                names.push(id.to_string());
                found.push(character);
            }
            None => continue,
        }
    }
    // Alphabetical by id: the picker's order must not depend on the order
    // the filesystem happened to hand the folders over in.
    found.sort_by(|a, b| a.id.cmp(&b.id));
    if found.len() > MAX_ROSTER {
        issues.push(format!(
            "{} characters installed; only the first {MAX_ROSTER} are shown",
            found.len()
        ));
        found.truncate(MAX_ROSTER);
    }
    (found, issues)
}

fn load_character(dir: &Path, id: &str, issues: &mut Vec<String>) -> Option<Character> {
    let meta_path = dir.join("metadata.json");
    let head = match peek(&meta_path, MAX_META_BYTES, MAX_META_BYTES as usize) {
        Ok(bytes) => bytes,
        Err(why) => {
            issues.push(format!("{id}: metadata.json — {why}"));
            return None;
        }
    };
    let text = match String::from_utf8(head) {
        Ok(text) => text,
        Err(_) => {
            issues.push(format!("{id}: metadata.json is not UTF-8"));
            return None;
        }
    };
    let meta = match parse_metadata(&text) {
        Ok(meta) => meta,
        Err(why) => {
            issues.push(format!("{id}: metadata.json — {why}"));
            return None;
        }
    };
    let mut own: Vec<String> = Vec::new();
    let mut character = finalize(id, meta, &mut own);

    // Images. A character with no usable standing art has nothing to show
    // on the board, so it is not offered at all.
    for (slot, file) in [(0usize, "standing_right.png"), (1, "standing_left.png")] {
        let path = dir.join("images").join(file);
        if !path.exists() {
            continue;
        }
        match image_ok(&path) {
            Ok(()) => character.standing[slot] = Some(asset_path(id, "images", file)),
            Err(why) => own.push(format!("{id}/{file}: {why}")),
        }
    }
    if character.standing.iter().all(Option::is_none) {
        issues.push(format!(
            "{id}: no usable standing art (images/standing_right.png or images/standing_left.png)"
        ));
        issues.append(&mut own);
        return None;
    }
    for (file, slot) in [
        ("cutin.png", None),
        ("icon.png", None),
        ("win.png", Some(0usize)),
        ("lose.png", Some(1)),
    ] {
        let path = dir.join("images").join(file);
        if !path.exists() {
            continue;
        }
        match image_ok(&path) {
            Ok(()) => {
                let asset = asset_path(id, "images", file);
                match (file, slot) {
                    (_, Some(i)) => character.result[i] = Some(asset),
                    ("icon.png", _) => character.icon = Some(asset),
                    _ => character.cutin = Some(asset),
                }
            }
            Err(why) => own.push(format!("{id}/{file}: {why}")),
        }
    }

    // Voices. All optional, so a missing one is silence rather than a
    // complaint — but a file that looks like a *misspelled* voice is worth
    // saying out loud, because it is indistinguishable from silence.
    let voices = dir.join("voices");
    let take = |stem: &str, own: &mut Vec<String>| {
        let file = format!("{stem}.wav");
        let path = voices.join(&file);
        if !path.exists() {
            return None;
        }
        match wav_ok(&path) {
            Ok(()) => Some(asset_path(id, "voices", &file)),
            Err(why) => {
                own.push(format!("{id}/voices/{file}: {why}"));
                None
            }
        }
    };
    for (i, kind) in VoiceKind::ALL.iter().enumerate() {
        character.voices[i] = take(kind.file_stem(), &mut own);
    }
    for n in 1..=COUNT_MAX {
        character.counts[(n - 1) as usize] = take(&count_stem(n), &mut own);
    }
    if let Ok(extra) = std::fs::read_dir(&voices) {
        let known = voice_file_stems();
        for entry in extra.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let stem = name.trim_end_matches(".wav");
            if known.iter().any(|k| k == stem) {
                continue;
            }
            if let Some(meant) = nearest_voice_name(stem) {
                own.push(format!(
                    "{id}/voices/{name}: not a voice this game plays — did you mean {meant}.wav?"
                ));
            }
        }
    }

    character.issues = own.clone();
    issues.append(&mut own);
    Some(character)
}

fn image_ok(path: &Path) -> Result<(), String> {
    let head = peek(path, MAX_IMAGE_BYTES, 24)?;
    match png_dimensions(&head) {
        None => Err("not a PNG (the extension says otherwise)".to_string()),
        Some((w, h)) if w == 0 || h == 0 || w > MAX_IMAGE_EDGE || h > MAX_IMAGE_EDGE => {
            Err(format!("{w}x{h} is outside 1..={MAX_IMAGE_EDGE} per side"))
        }
        Some(_) => Ok(()),
    }
}

fn asset_path(id: &str, folder: &str, file: &str) -> String {
    format!("characters/{id}/{folder}/{file}")
}

// ---------------------------------------------------------------------------
// Choosing
// ---------------------------------------------------------------------------

/// The character the opponent plays. Deterministic on purpose: the same
/// stage always fields the same face, so a player can learn it, and a
/// replay of the same match looks the same.
fn opponent_for(roster: &CharacterRoster, mode: &GameMode, player: Option<usize>) -> Option<usize> {
    let n = roster.0.len();
    if n == 0 {
        return None;
    }
    let key = match *mode {
        GameMode::VsCpu { stage } | GameMode::ZoneBattle { stage } => stage as usize,
        GameMode::Custom => 0,
        _ => return None,
    };
    let pick = key % n;
    // Prefer not to mirror the player, but never at the cost of returning
    // nobody when they are the only one installed.
    if Some(pick) == player && n > 1 {
        return Some((pick + 1) % n);
    }
    Some(pick)
}

/// Fill in whatever the picker did not. Every entry into `Playing` goes
/// through this, including the ones that skip the picker entirely — a
/// restart, and `BEVYTRIS_SCREEN=playing`.
fn resolve_match_characters(
    roster: Res<CharacterRoster>,
    mode: Res<GameMode>,
    settings: Res<GameSettings>,
    mut chosen: ResMut<MatchCharacters>,
    server: Res<AssetServer>,
    mut assets: ResMut<CharacterAssets>,
) {
    if roster.0.is_empty() {
        return;
    }
    if chosen.sides[0].is_none() {
        chosen.sides[0] = roster
            .index_of(&settings.character)
            .or(Some(0));
    }
    if chosen.sides[1].is_none() {
        chosen.sides[1] = opponent_for(&roster, &mode, chosen.sides[0]);
    }
    // Loading is idempotent and cached, so doing this per match costs one
    // hash lookup per file and buys a voice that is ready when the first
    // line lands rather than a second later.
    for side in chosen.sides.iter().flatten() {
        if let Some(character) = roster.get(*side) {
            preload(character, &server, &mut assets);
        }
    }
}

fn preload(character: &Character, server: &AssetServer, assets: &mut CharacterAssets) {
    for path in character.standing.iter().flatten() {
        assets.image(server, path);
    }
    for path in [&character.cutin, &character.icon].into_iter().flatten() {
        assets.image(server, path);
    }
    for path in character.result.iter().flatten() {
        assets.image(server, path);
    }
    for path in character
        .voices
        .iter()
        .chain(character.counts.iter())
        .flatten()
    {
        assets.sound(server, path);
    }
}

// ---------------------------------------------------------------------------
// The portrait
// ---------------------------------------------------------------------------

/// Draw order inside a board. The backdrop is 0.5 and the cell grid is
/// [`CELL_Z`], so a portrait here is *inside* the field: the stack covers
/// it as it builds, and the HUD (all of which is at 2.0) stays on top.
/// That is the whole trick — it reads as part of the board rather than as
/// a sticker over it.
const PORTRAIT_Z: f32 = 0.55;

/// How much of the field's height the art fills.
const PORTRAIT_FILL: f32 = 0.94;

#[derive(Component)]
struct Portrait {
    slot: usize,
    /// Field size in world units, for the aspect fit.
    board: Vec2,
    /// Whether the fit has happened yet — it cannot until the image has
    /// loaded and its real dimensions are known.
    fitted: bool,
}

/// Chained after `setup_board_visuals`, which is where `BoardTheme` comes
/// from.
pub fn spawn_portraits(
    mut commands: Commands,
    roster: Res<CharacterRoster>,
    chosen: Res<MatchCharacters>,
    server: Res<AssetServer>,
    mut assets: ResMut<CharacterAssets>,
    boards: Query<(Entity, &BoardIndex, &BoardTheme)>,
) {
    for (entity, index, theme) in &boards {
        let slot = index.0.min(1);
        let Some(character) = chosen.sides[slot].and_then(|i| roster.get(i)) else {
            continue;
        };
        let Some(path) = character.standing_for(slot) else {
            continue;
        };
        let image = assets.image(&server, path);
        let board = Vec2::new(theme.cell * 10.0, theme.cell * 20.0);
        let height = board.y * PORTRAIT_FILL;
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Sprite {
                    image,
                    // A guess until the image loads and `fit_portraits`
                    // measures it; without one, a single frame of the
                    // texture's native pixel size flashes across the
                    // screen.
                    custom_size: Some(Vec2::new(height * 0.5, height)),
                    // The scenes behind this are bloom-boosted, so an
                    // untreated sprite reads as a grey sticker.
                    color: emissive(
                        Color::srgba(1.0, 1.0, 1.0, character.portrait_alpha),
                        1.15,
                    ),
                    ..default()
                },
                Anchor::BOTTOM_CENTER,
                Transform::from_xyz(0.0, -board.y / 2.0, PORTRAIT_Z),
                Portrait {
                    slot,
                    board,
                    fitted: false,
                },
            ));
        });
    }
}

/// Fit the art inside its own field once its real size is known: by
/// height, then clamped by width so a wide image cannot spill over the
/// frame and out into the next board.
fn fit_portraits(
    images: Res<Assets<Image>>,
    mut portraits: Query<(&mut Portrait, &mut Sprite, &mut Transform)>,
) {
    for (mut portrait, mut sprite, mut transform) in &mut portraits {
        if portrait.fitted {
            continue;
        }
        let Some(image) = images.get(&sprite.image) else {
            continue;
        };
        let size = image.size_f32();
        if size.x <= 0.0 || size.y <= 0.0 {
            portrait.fitted = true;
            continue;
        }
        let scale = (portrait.board.y * PORTRAIT_FILL / size.y).min(portrait.board.x / size.x);
        let fitted = size * scale;
        sprite.custom_size = Some(fitted);
        // Stand against the outer edge of the field, so the character is
        // looking across their own board rather than out of the screen.
        let slack = ((portrait.board.x - fitted.x) / 2.0).max(0.0);
        transform.translation.x = if portrait.slot == 1 { slack } else { -slack };
        portrait.fitted = true;
    }
}

// ---------------------------------------------------------------------------
// The cut-in
// ---------------------------------------------------------------------------

/// Where a cut-in sits: *behind* the boards.
///
/// This has been three things. A card across the middle of the screen
/// covered the field for a second every time the player earned it. A small
/// strip in the corner fixed that and looked like a phone notification.
/// So it is scenery now — big, behind everything, part of the background
/// rather than a thing pasted over the game. That also decides the
/// technology: bevy_ui draws after the whole 2D pass, so a UI node is in
/// front of every sprite whatever its z. Only a world-space sprite can be
/// behind a board.
///
/// The band is wide open. Background scenes live at z -18 and below, and
/// the shallowest thing a board draws is its own opaque backdrop at 0.5.
const CUTIN_Z: f32 = -1.0;

/// How long the whole thing lasts, and how much of that is the fade in and
/// out. Behind the field it costs the player nothing, so it can hold.
const CUTIN_LIFE: f32 = 2.0;
const CUTIN_FADE_IN: f32 = 0.16;
const CUTIN_FADE_OUT: f32 = 0.45;

/// The sweep across the screen, in world units, and how long the entry
/// takes to settle.
///
/// A cut-in that only fades wastes the fact that most of it is behind a
/// board: whatever the fields cover at the moment it appears is never seen
/// at all. Moving it means the whole image passes through the gaps — the
/// boards become a shutter the art travels behind rather than a lid over
/// it. It enters from its owner's side, hard and fast, then keeps drifting
/// the same way for the rest of its life; reversing at the end would undo
/// the reveal it just paid for.
const CUTIN_ENTER: f32 = 560.0;
const CUTIN_ENTER_SECS: f32 = 0.34;
const CUTIN_DRIFT_X: f32 = 260.0;

/// The 1280x720 composition the camera is built around (`setup_camera`
/// uses `ScalingMode::AutoMin` on exactly this), inset so a cut-in never
/// runs off the edge.
const CUTIN_BOX: Vec2 = Vec2::new(1240.0, 690.0);

/// How solid the art gets at its peak: completely.
///
/// The pack's own art is the point, and blending it with whatever
/// background scene happens to be running behind it only made both look
/// washed out. Transparency is now only used for the way in and the way
/// out — while it holds, this is the picture, not a tint over one. (A PNG's
/// own alpha channel is untouched either way, so art drawn on a
/// transparent canvas still shows the scene through its own gaps.)
///
/// A board's backdrop is opaque, so in versus the two fields hide most of
/// it and it shows through the middle gap and around the edges; in solo,
/// with one field and a lot more spare screen, it reads clearly. Either
/// way it is never in front of anything the player is using.
const CUTIN_ALPHA: f32 = 1.0;

/// How much of the move's colour is laid over the art.
///
/// A straight multiply by the tint was fine at 70% opacity, where the
/// scene behind diluted it. At full opacity it repaints the whole image in
/// one hue and the pack's own colours are gone, so the tint is mixed
/// towards white first: the art keeps its palette and just leans in the
/// direction of the move.
///
/// Deliberately no HDR boost. Everything else on this screen is neon line
/// art that is *supposed* to bloom; a character drawn at full-screen size
/// and then bloomed washes out the HUD sitting on top of it, and a
/// background that outshines the numbers the player is reading has
/// misunderstood its job.
const CUTIN_WASH: f32 = 0.4;

/// How far it swells while it holds. Just enough that it is breathing.
const CUTIN_DRIFT: f32 = 0.05;

/// A zone release also fires the gold line tally, which is the bigger
/// moment; let it peak first.
const CUTIN_DELAY_ZONE: f32 = 0.45;

#[derive(Component)]
struct CutIn {
    life: f32,
    /// Size at full bloom, before the drift is applied.
    size: Vec2,
    /// The move's colour. Kept here rather than read back off the sprite:
    /// the tint is written through `emissive`, and recovering it from a
    /// value that has already been multiplied would multiply it again on
    /// every frame.
    tint: Color,
    /// Where the sweep passes through, and which way it is going: +1 for a
    /// character on the left board (entering from the left, moving right),
    /// -1 for one on the right.
    anchor: f32,
    dir: f32,
    /// How loudly the moment on screen insists. Kept on the entity rather
    /// than in the queue resource so it cannot outlive it: the cut-in is
    /// `DespawnOnExit(AppState::Playing)`, and a flag in a resource would
    /// stay latched when a restart takes the entity away mid-slide —
    /// silently suppressing every later cut-in for the rest of the run.
    priority: u8,
}

#[derive(Resource, Default)]
struct CutInQueue {
    pending: Option<(f32, usize, CutInKind)>,
}

#[derive(Message)]
struct ShowCutIn {
    slot: usize,
    kind: CutInKind,
    delay: f32,
}

/// Ask a character to say something.
#[derive(Message)]
pub struct PlayVoice {
    /// Which board is speaking, for the stereo placement and the
    /// one-line-at-a-time rule.
    slot: usize,
    /// Who is speaking, when that is not yet a question of boards — the
    /// picker plays a line for a character nobody has been assigned to.
    who: Option<usize>,
    line: VoiceLine,
}

impl PlayVoice {
    fn on(slot: usize, kind: VoiceKind) -> Self {
        Self {
            slot,
            who: None,
            line: VoiceLine::Kind(kind),
        }
    }

    /// A line from a named roster entry, for the picker: no match has
    /// started, so there is no board to look the character up from.
    pub fn from_roster(index: usize, kind: VoiceKind) -> Self {
        Self {
            slot: 0,
            who: Some(index),
            line: VoiceLine::Kind(kind),
        }
    }
}

fn queue_cutins(
    mut reader: MessageReader<ShowCutIn>,
    mut queue: ResMut<CutInQueue>,
    live: Query<&CutIn>,
) {
    let active = live.iter().map(|c| c.priority).max();
    for msg in reader.read() {
        let incoming = msg.kind.priority();
        // A smaller moment never interrupts a bigger one, and never waits
        // for it either — by the time it got its turn it would be
        // celebrating something the player has forgotten.
        if active.is_some_and(|active| active >= incoming) {
            continue;
        }
        if queue
            .pending
            .is_some_and(|(_, _, kind)| kind.priority() >= incoming)
        {
            continue;
        }
        queue.pending = Some((msg.delay, msg.slot, msg.kind));
    }
}

/// The move's colour, leaned towards white so it colours the art instead of
/// replacing it, at the given opacity. Linear rather than sRGB because that
/// is the space the renderer multiplies in.
fn cutin_color(tint: Color, alpha: f32) -> Color {
    let t = tint.to_linear();
    let lean = |c: f32| 1.0 - CUTIN_WASH * (1.0 - c);
    Color::LinearRgba(bevy::color::LinearRgba {
        red: lean(t.red),
        green: lean(t.green),
        blue: lean(t.blue),
        alpha,
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_cutins(
    mut commands: Commands,
    time: Res<Time>,
    mut queue: ResMut<CutInQueue>,
    roster: Res<CharacterRoster>,
    chosen: Res<MatchCharacters>,
    server: Res<AssetServer>,
    mut assets: ResMut<CharacterAssets>,
    images: Res<Assets<Image>>,
    boards: Query<(&BoardIndex, &Transform)>,
    live: Query<Entity, With<CutIn>>,
) {
    let Some((delay, slot, kind)) = queue.pending else {
        return;
    };
    if delay > 0.0 {
        queue.pending = Some((delay - time.delta_secs(), slot, kind));
        return;
    }
    queue.pending = None;
    // `slot` is a board, not a roster entry — the two only agree when the
    // player happens to have picked the first character installed.
    let slot = slot.min(1);
    let Some(character) = chosen.sides[slot].and_then(|i| roster.get(i)) else {
        return;
    };
    let Some(path) = character
        .cutin
        .as_deref()
        .or_else(|| character.standing_for(slot))
    else {
        return;
    };
    let image = assets.image(&server, path);
    for entity in &live {
        commands.entity(entity).despawn();
    }
    debug!("cut-in {kind:?} for {} on slot {slot}", character.id);

    // Fit inside the composition, preserving whatever aspect the pack drew.
    // The art is preloaded when the match starts, so this is measured once
    // here rather than deferred the way the board portrait has to be.
    let native = images
        .get(&image)
        .map(|i| i.size_f32())
        .filter(|s| s.x > 0.0 && s.y > 0.0)
        .unwrap_or(Vec2::new(1024.0, 512.0));
    let scale = (CUTIN_BOX.x / native.x).min(CUTIN_BOX.y / native.y);
    let size = native * scale;
    // Centred on its owner's board, then pulled back on screen: wide art
    // ends up middle-of-screen, a narrow portrait stands over the board it
    // belongs to.
    let anchor = boards
        .iter()
        .find(|(index, _)| index.0.min(1) == slot)
        .map(|(_, tf)| tf.translation.x)
        .unwrap_or(0.0);
    let room = (CUTIN_BOX.x - size.x).max(0.0) / 2.0;
    let anchor = anchor.clamp(-room, room);
    // Enter from the side the character's own board is on.
    let dir = if slot == 1 { -1.0 } else { 1.0 };
    commands.spawn((
        Sprite {
            image,
            custom_size: Some(size),
            // The move's colour, so a glance says which one it was. Pushed
            // into HDR because everything else on this screen blooms.
            color: cutin_color(kind.tint(), 0.0),
            ..default()
        },
        Transform::from_xyz(anchor - dir * CUTIN_ENTER, 0.0, CUTIN_Z),
        CutIn {
            life: CUTIN_LIFE,
            size,
            tint: kind.tint(),
            anchor,
            dir,
            priority: kind.priority(),
        },
        DespawnOnExit(AppState::Playing),
    ));
}

/// Bloom in, drift, fade out. Ungated on purpose, like every other
/// transient in this game: a cut-in caught by a pause finishes rather than
/// freezing halfway.
fn update_cutins(
    mut commands: Commands,
    time: Res<Time>,
    mut cutins: Query<(Entity, &mut CutIn, &mut Sprite, &mut Transform)>,
) {
    for (entity, mut cutin, mut sprite, mut transform) in &mut cutins {
        cutin.life -= time.delta_secs();
        if cutin.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        // Absolute seconds, not fractions of the life: the fade should look
        // the same however long the art holds for.
        let elapsed = CUTIN_LIFE - cutin.life;
        let fade = (elapsed / CUTIN_FADE_IN).min(cutin.life / CUTIN_FADE_OUT).min(1.0);
        sprite.color = cutin_color(cutin.tint, CUTIN_ALPHA * fade);
        // Swelling slowly is what keeps it from reading as a still image.
        let progress = elapsed / CUTIN_LIFE;
        sprite.custom_size = Some(cutin.size * (1.0 + CUTIN_DRIFT * progress));
        // Cubic ease-out for the entry, then a steady drift onward. The two
        // are summed rather than sequenced so there is no seam where the
        // slide hands over to the drift.
        let settle = (elapsed / CUTIN_ENTER_SECS).min(1.0);
        let remaining = (1.0 - settle).powi(3);
        transform.translation.x =
            cutin.anchor + cutin.dir * (CUTIN_DRIFT_X * progress - CUTIN_ENTER * remaining);
    }
}

// ---------------------------------------------------------------------------
// The voice
// ---------------------------------------------------------------------------

/// One sink per side. A character talking over themselves is the fastest
/// way to make a voice pack unbearable, so a line either cuts the previous
/// one off or is dropped.
#[derive(Component)]
struct VoiceSink {
    slot: usize,
    priority: u8,
    /// Watchdog. A sink whose asset never loads would otherwise hold its
    /// slot shut forever.
    life: f32,
}

/// Speech against one-shots.
///
/// The sound effects in this game are peak-normalized to -1 dBFS impacts;
/// a spoken line is a long, low-crest-factor waveform whose *average*
/// level is far below its peak, and macOS `say` output in particular sits
/// well under the effects. Matching them by peak, which is what the shared
/// `sfx_linear()` does, leaves the voice audibly quieter than the lock
/// sound. This is the make-up gain for that, and it is applied before the
/// per-character `voice_gain` so a pack can still trim itself.
const VOICE_MAKEUP: f32 = 2.6;

/// The loudest a line may end up, after every gain in the chain. Above
/// this the mix starts clipping into the drums.
const VOICE_CEILING: f32 = 2.2;

/// How much quieter the other board is. Enough to place it across the
/// room, not enough to lose the words.
const OPPONENT_GAIN: f32 = 0.8;

/// How long a line may hold its slot before the watchdog frees it.
const VOICE_WATCHDOG: f32 = 4.0;
/// The quieter clears say something at most this often, per side.
const CHATTY_COOLDOWN: f32 = 2.5;

#[derive(Resource, Default)]
struct VoiceCooldown([f32; 2]);

fn tick_voice_cooldown(time: Res<Time>, mut cooldown: ResMut<VoiceCooldown>) {
    for slot in &mut cooldown.0 {
        *slot = (*slot - time.delta_secs()).max(0.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn play_voices(
    mut commands: Commands,
    mut reader: MessageReader<PlayVoice>,
    roster: Res<CharacterRoster>,
    chosen: Res<MatchCharacters>,
    settings: Res<GameSettings>,
    server: Res<AssetServer>,
    mut assets: ResMut<CharacterAssets>,
    mut cooldown: ResMut<VoiceCooldown>,
    sinks: Query<(Entity, &VoiceSink)>,
) {
    let base = settings.sfx_linear();
    // Commands are deferred, so a sink spawned earlier in this same run is
    // not in the query yet. Two events for one board in one frame is
    // ordinary (a T-spin that clears nothing while garbage lands), and
    // without this both would play at once over each other.
    let mut spawned: [Option<(Entity, u8)>; 2] = [None, None];
    for msg in reader.read() {
        let slot = msg.slot.min(1);
        let Some(character) = msg
            .who
            .or(chosen.sides[slot])
            .and_then(|i| roster.get(i))
        else {
            continue;
        };
        let Some((path, counted)) = character.line_clip(msg.line) else {
            continue;
        };
        let priority = msg.line.priority();
        // The small talk gets rationed; the big moments never do. A
        // spoken number is neither: it is one of a run, and a rationed
        // count would say "two ... six ... " with the middle missing.
        if priority <= 2 && !counted {
            if cooldown.0[slot] > 0.0 {
                continue;
            }
            cooldown.0[slot] = CHATTY_COOLDOWN;
        }
        // A louder line arriving in the same frame still wins, but it has
        // to take the one already queued with it.
        if let Some((queued, active)) = spawned[slot] {
            if active >= priority {
                continue;
            }
            commands.entity(queued).despawn();
            spawned[slot] = None;
        }
        let mut busy = None;
        for (entity, sink) in &sinks {
            if sink.slot == slot {
                busy = Some((entity, sink.priority));
            }
        }
        if let Some((entity, active)) = busy {
            if active > priority {
                continue;
            }
            commands.entity(entity).despawn();
        }
        // The opponent is across the room: audible, but not competing with
        // the player's own reactions.
        let side_gain = if slot == 0 { 1.0 } else { OPPONENT_GAIN };
        let volume = (base * VOICE_MAKEUP * character.voice_gain * side_gain).min(VOICE_CEILING);
        if volume <= 0.001 {
            continue;
        }
        let handle = assets.sound(&server, path);
        // Whether a line actually played is otherwise unobservable, and
        // "my voice pack is silent" is the question a pack author will ask
        // first: RUST_LOG=bevytris=debug answers it.
        debug!(
            "voice {:?} for {} on slot {slot} at {volume:.2} ({path})",
            msg.line, character.id
        );
        let sink = commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
                VoiceSink {
                    slot,
                    priority,
                    life: VOICE_WATCHDOG,
                },
                DespawnOnExit(AppState::Playing),
            ))
            .id();
        spawned[slot] = Some((sink, priority));
    }
}

fn update_voice_sinks(
    mut commands: Commands,
    time: Res<Time>,
    mut sinks: Query<(Entity, &mut VoiceSink)>,
) {
    for (entity, mut sink) in &mut sinks {
        sink.life -= time.delta_secs();
        if sink.life <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Hooking the game up to all of the above
// ---------------------------------------------------------------------------

/// The board events worth a reaction. A second reader over the same
/// message stream as `effects.rs` — messages are broadcast, so this costs
/// that module nothing.
fn character_events(
    mut reader: MessageReader<BoardEvent>,
    mode: Res<GameMode>,
    boards: Query<&BoardIndex>,
    mut voices: MessageWriter<PlayVoice>,
    mut cutins: MessageWriter<ShowCutIn>,
) {
    let two_boards = matches!(
        *mode,
        GameMode::VsCpu { .. } | GameMode::ZoneBattle { .. } | GameMode::Custom
    );
    for msg in reader.read() {
        let slot = boards
            .get(msg.board)
            .map(|index| index.0.min(1))
            .unwrap_or(msg.index.min(1));
        if let Some(line) = voice_for(&msg.event, two_boards) {
            voices.write(PlayVoice {
                slot,
                who: None,
                line,
            });
        }
        if let Some(kind) = cutin_for(&msg.event) {
            cutins.write(ShowCutIn {
                slot,
                kind,
                delay: if kind == CutInKind::ZoneRelease {
                    CUTIN_DELAY_ZONE
                } else {
                    0.0
                },
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Being in trouble
// ---------------------------------------------------------------------------

/// How far the danger meter has to climb before a character says something
/// about it, and how far it has to fall before they will say it again.
///
/// The gap between the two is the whole point. Danger is a smoothed level
/// rather than an event, so a stack parked on a single threshold would
/// have the character chanting about it; this way the trouble has to
/// genuinely ease before it counts as trouble again.
const PINCH_ON: f32 = 0.72;
const PINCH_OFF: f32 = 0.45;

/// Whether each side is currently considered to be in trouble.
#[derive(Resource, Default)]
struct PinchLatch([bool; 2]);

fn pinch_voice(
    danger: Res<crate::effects::DangerLevel>,
    mut latch: ResMut<PinchLatch>,
    boards: Query<&BoardIndex>,
    mut voices: MessageWriter<PlayVoice>,
) {
    for index in &boards {
        let slot = index.0.min(1);
        let level = danger.0[slot];
        if latch.0[slot] {
            latch.0[slot] = level >= PINCH_OFF;
        } else if level >= PINCH_ON {
            latch.0[slot] = true;
            voices.write(PlayVoice::on(slot, VoiceKind::Pinch));
        }
    }
}

/// A new round starts from a clear board, so it starts out of trouble.
fn clear_pinch(mut latch: ResMut<PinchLatch>) {
    *latch = PinchLatch::default();
}

/// Big WIN / LOSE lettering over a board, and the pose that goes with it.
///
/// World space rather than UI, because it belongs to the board it is about
/// — it rides the same `BoardKick` as everything else on that board, and
/// it sits behind the result overlay the way the field does.
const RESULT_TEXT_Z: f32 = 2.5;

/// The pose is the point at this moment, so it stops being scenery: still
/// translucent enough to read the stack through, far brighter than the
/// 0.20 it plays at.
const RESULT_ALPHA: f32 = 0.85;

/// And it comes out in front of the field for the same reason. A portrait
/// spends the match at [`PORTRAIT_Z`], under [`CELL_Z`], which is exactly
/// right while it is scenery — but a match ends with the board at its
/// fullest, so a win pose left back there is a win pose buried behind the
/// stack that just won. This sits over the cells and under
/// [`RESULT_TEXT_Z`], so the WIN/LOSE lettering still reads.
const RESULT_PORTRAIT_Z: f32 = 2.2;

// The portrait is scenery during the match and the subject of the screen
// after it, and those are opposite sides of the playfield. Cheap to state,
// silent to get wrong, so state it where a wrong edit cannot compile.
const _: () = assert!(PORTRAIT_Z < CELL_Z);
const _: () = assert!(RESULT_PORTRAIT_Z > CELL_Z);
const _: () = assert!(RESULT_PORTRAIT_Z < RESULT_TEXT_Z);

#[derive(Component)]
struct ResultBanner;

/// Did board 0 win? `None` while a match is still undecided.
fn player_won(
    result: Option<&crate::session::SessionResult>,
    last: Option<&crate::session::LastRound>,
) -> Option<bool> {
    use crate::session::SessionResult;
    match result {
        Some(SessionResult::VsWin { winner }) => Some(*winner == 0),
        Some(SessionResult::RaceDone) => Some(true),
        Some(SessionResult::SoloOver) => Some(false),
        None => last.map(|round| round.winner == 0),
    }
}

#[allow(clippy::too_many_arguments)]
fn show_result(
    mut commands: Commands,
    result: Option<Res<crate::session::SessionResult>>,
    last: Option<Res<crate::session::LastRound>>,
    roster: Res<CharacterRoster>,
    chosen: Res<MatchCharacters>,
    locale: Res<Locale>,
    server: Res<AssetServer>,
    mut assets: ResMut<CharacterAssets>,
    boards: Query<(Entity, &BoardIndex, &BoardTheme)>,
    mut portraits: Query<(&mut Portrait, &mut Sprite, &mut Transform)>,
    children: Query<&Children>,
) {
    let Some(player_won) = player_won(result.as_deref(), last.as_deref()) else {
        return;
    };
    let s = locale.s();
    for (entity, index, theme) in &boards {
        let slot = index.0.min(1);
        let won = (slot == 0) == player_won;
        let Some(character) = chosen.sides[slot].and_then(|i| roster.get(i)) else {
            continue;
        };
        // Swap the pose in place: the portrait is a child of this board.
        if let Ok(kids) = children.get(entity) {
            for kid in kids {
                if let Ok((mut portrait, mut sprite, mut transform)) = portraits.get_mut(*kid) {
                    if let Some(path) = character.result_art(slot, won) {
                        sprite.image = assets.image(&server, path);
                    }
                    sprite.color = emissive(Color::srgba(1.0, 1.0, 1.0, RESULT_ALPHA), 1.15);
                    transform.translation.z = RESULT_PORTRAIT_Z;
                    // Re-measure: the pose is very likely a different shape.
                    portrait.fitted = false;
                }
            }
        }
        let (label, tint) = if won {
            (s.match_win, Color::srgb(0.4, 1.0, 0.7))
        } else {
            (s.match_lose, Color::srgb(1.0, 0.45, 0.45))
        };
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Text2d::new(label),
                TextFont {
                    font_size: FontSize::Px(theme.cell * 1.5),
                    ..default()
                },
                TextColor(emissive(tint, 2.0)),
                // High on the board: the result overlay puts its own
                // headline across the middle of the screen, and two lines
                // of result text on top of each other reads as a glitch.
                Transform::from_xyz(0.0, theme.cell * 6.0, RESULT_TEXT_Z),
                ResultBanner,
            ));
        });
    }
}

/// Put the pose back for the next round. A first-to-n match keeps the same
/// board entities across rounds, so nothing else would.
fn clear_result(
    mut commands: Commands,
    roster: Res<CharacterRoster>,
    chosen: Res<MatchCharacters>,
    server: Res<AssetServer>,
    mut assets: ResMut<CharacterAssets>,
    banners: Query<Entity, With<ResultBanner>>,
    mut portraits: Query<(&mut Portrait, &mut Sprite, &mut Transform)>,
) {
    for entity in &banners {
        commands.entity(entity).despawn();
    }
    for (mut portrait, mut sprite, mut transform) in &mut portraits {
        let Some(character) = chosen.sides[portrait.slot].and_then(|i| roster.get(i)) else {
            continue;
        };
        if let Some(path) = character.standing_for(portrait.slot) {
            sprite.image = assets.image(&server, path);
        }
        sprite.color = emissive(
            Color::srgba(1.0, 1.0, 1.0, character.portrait_alpha),
            1.15,
        );
        // Back inside the field, where a portrait belongs while it is scenery.
        transform.translation.z = PORTRAIT_Z;
        portrait.fitted = false;
    }
}

fn ready_voice(mut voices: MessageWriter<PlayVoice>) {
    voices.write(PlayVoice::on(0, VoiceKind::Ready));
}

/// Win and loss are resources rather than events, so they are read on the
/// way into the state that shows them.
fn outcome_voice(
    result: Option<Res<crate::session::SessionResult>>,
    last: Option<Res<crate::session::LastRound>>,
    mut voices: MessageWriter<PlayVoice>,
) {
    let Some(player_won) = player_won(result.as_deref(), last.as_deref()) else {
        return;
    };
    for slot in 0..2 {
        let won = (slot == 0) == player_won;
        voices.write(PlayVoice::on(
            slot,
            if won { VoiceKind::Win } else { VoiceKind::Lose },
        ));
    }
}

/// Characters with nothing but a name, for tests elsewhere in the crate
/// that need a roster without touching the filesystem.
#[cfg(test)]
pub fn test_roster(ids: &[&str]) -> Vec<Character> {
    ids.iter()
        .map(|id| {
            let mut character = finalize(id, CharacterMeta::default(), &mut Vec::new());
            character.standing[0] = Some(asset_path(id, "images", "standing_right.png"));
            character
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Auditioning a pack
// ---------------------------------------------------------------------------

/// `BEVYTRIS_CHARACTER_DEMO=1`: walk every voice and every cut-in on a
/// timer, alternating sides.
///
/// A pack author otherwise has to *play well* to hear their own work — a
/// perfect clear voice is a long wait for someone who just wants to know
/// whether the file name is right. This plays the lot in order, hands off.
#[derive(Resource)]
struct CharacterDemo {
    timer: Timer,
    step: usize,
}

fn start_demo(mut commands: Commands) {
    if std::env::var("BEVYTRIS_CHARACTER_DEMO").is_ok() {
        info!("character demo: playing every voice and cut-in in turn");
        commands.insert_resource(CharacterDemo {
            timer: Timer::from_seconds(1.4, TimerMode::Repeating),
            step: 0,
        });
    }
}

fn run_demo(
    time: Res<Time>,
    demo: Option<ResMut<CharacterDemo>>,
    mut voices: MessageWriter<PlayVoice>,
    mut cutins: MessageWriter<ShowCutIn>,
) {
    let Some(mut demo) = demo else { return };
    if !demo.timer.tick(time.delta()).just_finished() {
        return;
    }
    let step = demo.step;
    demo.step += 1;
    let slot = (step / 2) % 2;
    // The named voices, then the numbers, then the cut-ins, then round
    // again — so the log reads in the same order as the lists in this file.
    let voice_count = VoiceKind::ALL.len();
    let counts = COUNT_MAX as usize;
    let cutins_all = [
        CutInKind::Tetris,
        CutInKind::TSpinDouble,
        CutInKind::TSpinTriple,
        CutInKind::Counter,
        CutInKind::ZoneRelease,
        CutInKind::PerfectClear,
    ];
    let cycle = voice_count + counts + cutins_all.len();
    let at = step % cycle;
    if at < voice_count {
        voices.write(PlayVoice::on(slot, VoiceKind::ALL[at]));
    } else if at < voice_count + counts {
        voices.write(PlayVoice {
            slot,
            who: None,
            // Auditioning is about hearing the files, so no fallback: a
            // missing number should be an audible gap, not the combo line
            // twenty times over.
            line: VoiceLine::Count {
                n: (at - voice_count) as u32 + 1,
                instead: None,
            },
        });
    } else {
        cutins.write(ShowCutIn {
            slot,
            kind: cutins_all[at - voice_count - counts],
            delay: 0.0,
        });
    }
}

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        let (roster, issues) = scan_characters();
        // Every complaint goes to the log, because the picker's own notice
        // only carries a count and tells the player to look here.
        for issue in &issues {
            warn!("character pack: {issue}");
        }
        if roster.is_empty() {
            info!("no characters installed (assets/characters is empty or missing)");
        } else {
            info!(
                "characters: {}",
                roster
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        app.insert_resource(CharacterRoster(roster))
            .insert_resource(CharacterIssues(issues))
            .init_resource::<CharacterAssets>()
            .init_resource::<MatchCharacters>()
            .init_resource::<CutInQueue>()
            .init_resource::<VoiceCooldown>()
            .init_resource::<PinchLatch>()
            .add_message::<PlayVoice>()
            .add_message::<ShowCutIn>()
            .add_systems(
                OnEnter(AppState::Playing),
                (resolve_match_characters, |mut queue: ResMut<CutInQueue>| {
                    // A cut-in still waiting out its delay when the match
                    // ended has nothing left to celebrate.
                    queue.pending = None;
                }),
            )
            .add_systems(
                OnEnter(PlayState::Countdown),
                (ready_voice, clear_result, clear_pinch, start_demo),
            )
            .add_systems(OnEnter(PlayState::RoundOver), (outcome_voice, show_result))
            // No `show_result` here. A decided match puts the result
            // overlay up immediately, and that screen carries the poses
            // itself — swapping them onto the boards underneath as well
            // would draw each character twice, once dimmed behind the
            // overlay and once on it.
            .add_systems(OnEnter(PlayState::Finished), outcome_voice)
            .add_systems(
                Update,
                (character_events, pinch_voice).run_if(in_state(PlayState::Running)),
            )
            // Everything else is ungated, matching the other transients:
            // a line or a cut-in already in flight finishes rather than
            // being frozen by a pause or cut off by a state change.
            .add_systems(
                Update,
                (
                    fit_portraits,
                    run_demo,
                    queue_cutins,
                    spawn_cutins,
                    update_cutins,
                    play_voices,
                    update_voice_sinks,
                    tick_voice_cooldown,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(json: &str) -> Character {
        let parsed = parse_metadata(json).expect("valid json");
        let mut issues = Vec::new();
        finalize("test_id", parsed, &mut issues)
    }

    #[test]
    fn a_name_is_the_only_thing_a_pack_must_supply() {
        let c = meta(r#"{"display_name":"Alice"}"#);
        assert_eq!(c.display_name, "Alice");
        assert_eq!(c.ascii_name, "Alice");
        assert_eq!(c.flavor, "");
        assert_eq!(c.portrait_alpha, PORTRAIT_ALPHA_DEFAULT);
        assert_eq!(c.voice_gain, 1.0);
    }

    #[test]
    fn unknown_and_future_fields_still_load() {
        // A pack authored against a later version has to keep working for
        // the parts this build understands.
        let c = meta(r#"{"schema":9,"display_name":"Bob","hair_colour":"green"}"#);
        assert_eq!(c.display_name, "Bob");
        let parsed = parse_metadata(r#"{"schema":9,"display_name":"Bob"}"#).unwrap();
        let mut issues = Vec::new();
        finalize("bob", parsed, &mut issues);
        assert!(
            issues.iter().any(|i| i.contains("schema 9")),
            "a newer schema should be reported: {issues:?}"
        );
    }

    #[test]
    fn a_broken_file_is_an_error_not_a_panic() {
        for bad in [
            "",
            "{",
            "[]",
            "null",
            r#"["Alice"]"#,
            r#"{"display_name":}"#,
            r#"{"display_name":42}"#,
            r#"{"portrait_alpha":"loud"}"#,
        ] {
            assert!(parse_metadata(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn a_hostile_name_is_defanged() {
        // Bidi override, zero-width space, a newline, and far too long.
        let nasty = format!("\u{202E}Ali\u{200B}ce\nthe\tgreat {}", "x".repeat(200));
        let mut issues = Vec::new();
        let parsed = CharacterMeta {
            display_name: Some(nasty),
            ..Default::default()
        };
        let c = finalize("id", parsed, &mut issues);
        assert_eq!(c.display_name.chars().count(), 24);
        assert!(!c.display_name.contains('\u{202E}'));
        assert!(!c.display_name.contains('\u{200B}'));
        assert!(!c.display_name.contains('\n'));
        assert!(!c.display_name.contains('\t'));
    }

    #[test]
    fn a_name_that_sanitizes_to_nothing_falls_back_to_the_folder() {
        // Zero-width spaces, escaped so the source stays readable.
        let c = meta("{\"display_name\":\"\\u200b\\u200b\"}");
        assert_eq!(c.display_name, "TEST ID");
        assert_eq!(c.ascii_name, "TEST ID");
    }

    #[test]
    fn a_non_ascii_name_still_gets_an_ascii_one() {
        let c = meta(r#"{"display_name":"ダミー・ボブ","ascii_name":"DUMMY BOB"}"#);
        assert_eq!(c.display_name, "ダミー・ボブ");
        assert_eq!(c.ascii_name, "DUMMY BOB");
        // And with no ascii_name supplied, the folder name is the fallback
        // rather than an empty label.
        let c = meta(r#"{"display_name":"ダミー"}"#);
        assert_eq!(c.ascii_name, "TEST ID");
    }

    #[test]
    fn numbers_are_clamped_not_trusted() {
        let c = meta(r#"{"display_name":"a","portrait_alpha":99.0,"voice_gain":-5.0}"#);
        assert_eq!(c.portrait_alpha, 0.45);
        assert_eq!(c.voice_gain, 0.2);
        let c = meta(r#"{"display_name":"a","portrait_alpha":null}"#);
        assert_eq!(c.portrait_alpha, PORTRAIT_ALPHA_DEFAULT);
    }

    #[test]
    fn folder_names_are_screened() {
        for good in ["alice", "bob_2", "a-b", "A1"] {
            assert!(valid_id(good), "{good} should be allowed");
        }
        for bad in ["", "..", "a/b", " x", ".hidden", "has space", "é", &"x".repeat(49)] {
            assert!(!valid_id(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn a_png_header_is_read_and_a_jpeg_is_caught() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&512u32.to_be_bytes());
        png.extend_from_slice(&1024u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((512, 1024)));

        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1];
        assert_eq!(png_dimensions(&jpeg), None);
        assert_eq!(png_dimensions(b"not an image at all!!!!!!"), None);
    }

    #[test]
    fn only_wav_counts_as_audio() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0, 0, 0, 0]);
        wav.extend_from_slice(b"WAVE");
        assert!(is_riff_wave(&wav));
        assert!(!is_riff_wave(b"OggS\0\0\0\0\0\0\0\0"));
        assert!(!is_riff_wave(b"this is a text file"));
    }

    /// Write a WAV with the given format tag and chunk layout to a temp
    /// path, then run the real validator over it.
    fn check_wav(chunks: &[(&[u8; 4], Vec<u8>)], name: &str) -> Result<(), String> {
        let mut body: Vec<u8> = b"WAVE".to_vec();
        for (tag, payload) in chunks {
            body.extend_from_slice(*tag);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        let path = std::env::temp_dir().join(format!("bevytris_wav_{name}.wav"));
        std::fs::write(&path, &file).expect("temp write");
        let verdict = wav_ok(&path);
        let _ = std::fs::remove_file(&path);
        verdict
    }

    fn fmt_chunk(tag: u16) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&tag.to_le_bytes()); // format
        fmt.extend_from_slice(&1u16.to_le_bytes()); // channels
        fmt.extend_from_slice(&22050u32.to_le_bytes()); // rate
        fmt.extend_from_slice(&44100u32.to_le_bytes()); // byte rate
        fmt.extend_from_slice(&2u16.to_le_bytes()); // align
        fmt.extend_from_slice(&16u16.to_le_bytes()); // bits
        fmt
    }

    #[test]
    fn a_wav_that_would_crash_the_decoder_is_rejected() {
        // This one is not cosmetic: bevy hands the bytes to the decoder
        // with an unwrap, so a file that only *looks* like a WAV takes the
        // game down mid-match rather than going quiet.
        assert!(
            check_wav(
                &[(b"fmt ", fmt_chunk(WAVE_PCM)), (b"data", vec![0; 64])],
                "pcm"
            )
            .is_ok(),
            "ordinary 16-bit PCM has to pass"
        );
        // What `say` writes: junk padding between the header and the data.
        assert!(
            check_wav(
                &[
                    (b"JUNK", vec![0; 28]),
                    (b"fmt ", fmt_chunk(WAVE_PCM)),
                    (b"FLLR", vec![0; 4000]),
                    (b"data", vec![0; 64]),
                ],
                "padded"
            )
            .is_ok(),
            "padding chunks must be skipped, not tripped over"
        );
        assert!(
            check_wav(
                &[(b"fmt ", fmt_chunk(WAVE_IEEE_FLOAT)), (b"data", vec![0; 64])],
                "float"
            )
            .is_ok()
        );
        // mu-law: a real format, and one this build cannot decode.
        let mulaw = check_wav(&[(b"fmt ", fmt_chunk(7)), (b"data", vec![0; 64])], "mulaw");
        assert!(mulaw.is_err(), "mu-law should be refused");
        assert!(mulaw.unwrap_err().contains("PCM"));
        // Truncated download: the header and nothing behind it.
        assert!(check_wav(&[], "empty").is_err(), "a bare RIFF/WAVE stub");
        assert!(
            check_wav(&[(b"data", vec![0; 64])], "nofmt")
                .unwrap_err()
                .contains("fmt"),
        );
        assert!(
            check_wav(&[(b"fmt ", fmt_chunk(WAVE_PCM))], "nodata")
                .unwrap_err()
                .contains("data"),
        );
    }

    #[test]
    fn a_misspelled_voice_is_named() {
        let meant = |stem| nearest_voice_name(stem);
        assert_eq!(meant("tetriss").as_deref(), Some("tetris"));
        assert_eq!(meant("wins").as_deref(), Some("win"));
        assert_eq!(meant("t_spin").as_deref(), Some("tspin"));
        // The likeliest slip of all: dropping the zero from a number.
        assert_eq!(meant("count_3").as_deref(), Some("count_03"));
        assert_eq!(meant("background_music"), None);
    }

    #[test]
    fn a_single_facing_is_reused_for_both_sides() {
        let mut c = meta(r#"{"display_name":"a"}"#);
        c.standing[0] = Some("characters/a/images/standing_right.png".into());
        assert!(c.standing_for(0).unwrap().ends_with("standing_right.png"));
        assert!(c.standing_for(1).unwrap().ends_with("standing_right.png"));
    }

    fn clear(lines: u32, kind: ClearKind) -> GameEvent {
        GameEvent::Cleared(LineClear {
            lines,
            kind,
            b2b: false,
            combo: 0,
            perfect_clear: false,
            rows: Vec::new(),
            attack: 0,
            score_gained: 0,
        })
    }

    #[test]
    fn the_big_moments_get_a_cut_in_and_the_rest_do_not() {
        assert_eq!(cutin_for(&clear(4, ClearKind::Normal)), Some(CutInKind::Tetris));
        assert_eq!(
            cutin_for(&clear(2, ClearKind::TSpin)),
            Some(CutInKind::TSpinDouble)
        );
        assert_eq!(
            cutin_for(&clear(3, ClearKind::TSpin)),
            Some(CutInKind::TSpinTriple)
        );
        assert_eq!(cutin_for(&clear(1, ClearKind::TSpin)), None);
        assert_eq!(cutin_for(&clear(1, ClearKind::Normal)), None);
        assert_eq!(cutin_for(&clear(3, ClearKind::Normal)), None);
        assert_eq!(cutin_for(&GameEvent::ZoneEnded { lines: 4, attack: 4 }), None);
        assert_eq!(
            cutin_for(&GameEvent::ZoneEnded { lines: 5, attack: 6 }),
            Some(CutInKind::ZoneRelease)
        );

        let mut pc = clear(1, ClearKind::Normal);
        if let GameEvent::Cleared(c) = &mut pc {
            c.perfect_clear = true;
        }
        assert_eq!(cutin_for(&pc), Some(CutInKind::PerfectClear));
    }

    #[test]
    fn a_perfect_clear_outranks_everything_else() {
        let mut ranked: Vec<CutInKind> = vec![
            CutInKind::Tetris,
            CutInKind::PerfectClear,
            CutInKind::TSpinDouble,
            CutInKind::Counter,
            CutInKind::ZoneRelease,
            CutInKind::TSpinTriple,
        ];
        ranked.sort_by_key(|k| k.priority());
        assert_eq!(*ranked.last().unwrap(), CutInKind::PerfectClear);
        assert_eq!(*ranked.first().unwrap(), CutInKind::Tetris);
    }

    #[test]
    fn every_event_maps_to_at_most_one_voice() {
        let named = |event: &GameEvent, two| voice_for(event, two);
        assert_eq!(
            named(&clear(4, ClearKind::Normal), true),
            Some(VoiceLine::Kind(VoiceKind::Tetris))
        );
        assert_eq!(
            named(&clear(1, ClearKind::TSpinMini), true),
            Some(VoiceLine::Kind(VoiceKind::TSpin))
        );
        assert_eq!(
            named(&clear(1, ClearKind::Normal), true),
            Some(VoiceLine::Kind(VoiceKind::Clear))
        );
        assert_eq!(
            named(&GameEvent::GarbageRose { rows: 2 }, true),
            Some(VoiceLine::Kind(VoiceKind::Damage))
        );
        // One rising row is a fact of life, not an injury.
        assert_eq!(named(&GameEvent::GarbageRose { rows: 1 }, true), None);
        assert_eq!(
            named(&GameEvent::GarbageRose { rows: 4 }, true),
            Some(VoiceLine::Kind(VoiceKind::DamageHeavy))
        );
        assert_eq!(
            named(&GameEvent::ZoneReady, true),
            Some(VoiceLine::Kind(VoiceKind::ZoneReady))
        );
        assert_eq!(
            named(&GameEvent::ZoneActivated, true),
            Some(VoiceLine::Kind(VoiceKind::ZoneStart))
        );
        assert_eq!(named(&GameEvent::ZoneEnded { lines: 0, attack: 0 }, true), None);
        // Nothing to attack in a solo mode.
        let mut attacking = clear(2, ClearKind::Normal);
        if let GameEvent::Cleared(c) = &mut attacking {
            c.attack = 2;
        }
        assert_eq!(
            named(&attacking, true),
            Some(VoiceLine::Kind(VoiceKind::Attack))
        );
        assert_eq!(
            named(&attacking, false),
            Some(VoiceLine::Kind(VoiceKind::Clear))
        );
        // Chatter that has no line to say.
        assert_eq!(named(&GameEvent::Spawned, true), None);
        assert_eq!(named(&GameEvent::SoftDropStep, true), None);
        assert_eq!(named(&GameEvent::LevelUp { level: 3 }, true), None);
    }

    /// The whole point of the split: the size of the hit picks the line,
    /// and the boundary is where a tetris' worth lands at once.
    #[test]
    fn a_big_hit_and_a_small_one_do_not_sound_the_same() {
        let voice = |rows| voice_for(&GameEvent::GarbageRose { rows }, true);
        for rows in 2..HEAVY_GARBAGE_ROWS {
            assert_eq!(voice(rows), Some(VoiceLine::Kind(VoiceKind::Damage)), "{rows} rows");
        }
        for rows in [HEAVY_GARBAGE_ROWS, HEAVY_GARBAGE_ROWS + 1, 20] {
            assert_eq!(
                voice(rows),
                Some(VoiceLine::Kind(VoiceKind::DamageHeavy)),
                "{rows} rows"
            );
        }
    }

    /// A counter is only a counter once it saved you from something. One
    /// stray row cancelled is bookkeeping, and the cut-in waits for a
    /// tetris' worth on top of that.
    #[test]
    fn a_counter_has_to_be_worth_the_breath() {
        let event = |rows| GameEvent::Countered { rows };
        assert_eq!(voice_for(&event(1), true), None);
        assert_eq!(
            voice_for(&event(COUNTER_ROWS), true),
            Some(VoiceLine::Kind(VoiceKind::Counter))
        );
        assert_eq!(cutin_for(&event(COUNTER_CUTIN_ROWS - 1)), None);
        assert_eq!(
            cutin_for(&event(COUNTER_CUTIN_ROWS)),
            Some(CutInKind::Counter)
        );
    }

    /// A streak is counted out loud from its second clear, and a zone's
    /// bank from its first line — but a pack with no numbers must not go
    /// quiet where it used to speak.
    #[test]
    fn a_streak_is_counted_out_loud() {
        let combo = |n: u32| {
            let mut e = clear(1, ClearKind::Normal);
            if let GameEvent::Cleared(c) = &mut e {
                c.combo = n;
            }
            voice_for(&e, true)
        };
        // The first clear of a streak is not a streak yet.
        assert_eq!(combo(0), Some(VoiceLine::Kind(VoiceKind::Clear)));
        assert_eq!(
            combo(1),
            Some(VoiceLine::Count {
                n: 2,
                instead: Some(VoiceKind::Combo)
            })
        );
        assert_eq!(
            combo(6),
            Some(VoiceLine::Count {
                n: 7,
                instead: Some(VoiceKind::Combo)
            })
        );
        // A tetris is the bigger moment even mid-streak.
        let mut big = clear(4, ClearKind::Normal);
        if let GameEvent::Cleared(c) = &mut big {
            c.combo = 3;
        }
        assert_eq!(voice_for(&big, true), Some(VoiceLine::Kind(VoiceKind::Tetris)));

        assert_eq!(
            voice_for(&GameEvent::ZoneLines { banked: 2, total: 6 }, true),
            Some(VoiceLine::Count {
                n: 6,
                instead: None
            })
        );
    }

    /// Counting is a run, not chatter: each number replaces the one before
    /// it, and anything that actually matters replaces the number.
    #[test]
    fn a_number_gives_way_to_the_move_that_earned_it() {
        let count = VoiceLine::Count {
            n: 4,
            instead: None,
        };
        assert_eq!(count.priority(), VoiceLine::Kind(VoiceKind::Combo).priority());
        assert!(count.priority() < VoiceLine::Kind(VoiceKind::Tetris).priority());
        assert!(count.priority() > VoiceLine::Kind(VoiceKind::Clear).priority());
    }

    /// Numbers are their own family: a pack that ships them gets counted,
    /// and one that does not falls back to the line it always had.
    #[test]
    fn a_pack_without_numbers_still_says_something() {
        let mut c = meta(r#"{"display_name":"a"}"#);
        let combo = VoiceLine::Count {
            n: 3,
            instead: Some(VoiceKind::Combo),
        };
        assert_eq!(c.line_clip(combo), None);

        c.voices[VoiceKind::Combo as usize] = Some(asset_path("a", "voices", "combo.wav"));
        let (path, counted) = c.line_clip(combo).expect("the combo line");
        assert!(path.ends_with("combo.wav"));
        assert!(!counted, "the fallback is chatter and has to be rationed");

        c.counts[2] = Some(asset_path("a", "voices", "count_03.wav"));
        let (path, counted) = c.line_clip(combo).expect("the number");
        assert!(path.ends_with("count_03.wav"));
        assert!(counted);

        // Past the end of the numbers, and at zero, there is nothing to say.
        for n in [0, COUNT_MAX + 1] {
            assert_eq!(
                c.line_clip(VoiceLine::Count { n, instead: None }),
                None,
                "{n} is outside 1..={COUNT_MAX}"
            );
        }
    }

    /// A pack written before `damage_heavy.wav` existed still has something
    /// to say when it takes one; it just says the same thing as always.
    #[test]
    fn the_heavy_hit_falls_back_to_the_ordinary_one() {
        assert_eq!(meta(r#"{"display_name":"a"}"#).voice(VoiceKind::DamageHeavy), None);

        let mut c = meta(r#"{"display_name":"a"}"#);
        c.voices[VoiceKind::Damage as usize] = Some(asset_path("a", "voices", "damage.wav"));
        let heavy = c.voice(VoiceKind::DamageHeavy).expect("falls back");
        assert!(heavy.ends_with("damage.wav"));

        // Once it has its own clip, that is the one that plays.
        c.voices[VoiceKind::DamageHeavy as usize] =
            Some(asset_path("a", "voices", "damage_heavy.wav"));
        let heavy = c.voice(VoiceKind::DamageHeavy).expect("its own clip");
        assert!(heavy.ends_with("damage_heavy.wav"));
        // And the fallback only ever runs the one way.
        assert!(c.voice(VoiceKind::Damage).unwrap().ends_with("damage.wav"));
    }

    /// `voice()` indexes `voices` by the discriminant, so the array and the
    /// enum have to stay in the same order — a mismatch would quietly play
    /// the wrong clip rather than fail anywhere visible.
    #[test]
    fn the_voice_list_is_in_discriminant_order() {
        for (i, kind) in VoiceKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, i, "{kind:?} is out of order");
        }
    }

    #[test]
    fn a_loud_line_cuts_a_quiet_one_and_not_the_other_way() {
        assert!(VoiceKind::PerfectClear.priority() > VoiceKind::Clear.priority());
        assert!(VoiceKind::Win.priority() > VoiceKind::PerfectClear.priority());
        assert!(VoiceKind::Tetris.priority() > VoiceKind::Combo.priority());
        assert!(VoiceKind::DamageHeavy.priority() > VoiceKind::Damage.priority());
        assert!(VoiceKind::Counter.priority() > VoiceKind::Attack.priority());
        assert_eq!(VoiceKind::Clear.priority(), 1);
    }

    #[test]
    fn the_opponent_is_the_same_face_every_time_for_a_stage() {
        let roster = CharacterRoster(
            ["a", "b", "c"]
                .iter()
                .map(|id| {
                    let mut c = meta(r#"{"display_name":"x"}"#);
                    c.id = (*id).to_string();
                    c
                })
                .collect(),
        );
        let mode = GameMode::VsCpu { stage: 4 };
        assert_eq!(opponent_for(&roster, &mode, None), Some(1));
        assert_eq!(opponent_for(&roster, &mode, None), Some(1));
        // And it steps aside rather than mirroring the player.
        assert_eq!(opponent_for(&roster, &mode, Some(1)), Some(2));
        // Solo modes field nobody.
        assert_eq!(opponent_for(&roster, &GameMode::Single, None), None);
        // A roster of one has no choice but to mirror.
        let solo = CharacterRoster(vec![meta(r#"{"display_name":"x"}"#)]);
        assert_eq!(opponent_for(&solo, &mode, Some(0)), Some(0));
        // And an empty roster never hands out an index.
        assert_eq!(opponent_for(&CharacterRoster(Vec::new()), &mode, None), None);
    }

    #[test]
    fn voice_file_names_are_unique_and_lowercase() {
        let mut names: Vec<&str> = VoiceKind::ALL.iter().map(|k| k.file_stem()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two voices share a file name");
        for name in names {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name} is not a tidy file name"
            );
        }
    }
}
