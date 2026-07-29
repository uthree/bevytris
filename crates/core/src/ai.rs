//! Computer opponent, in the spirit of MisaMino and Cold Clear: a full
//! pathfinding move generator (soft-drop tucks, SRS spins), a beam search
//! over the preview queue, and a Dellacherie-style board evaluation
//! extended with attack, back-to-back and T-spin-slot awareness. Human
//! imperfections (speed, noise, blunders) are layered on per difficulty
//! stage. The Bevy layer executes the chosen plan with human-like timing;
//! this module is pure and synchronous.

use std::collections::{HashSet, VecDeque};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::board::{ActivePiece, BOARD_HEIGHT, BOARD_WIDTH, Board, VISIBLE_HEIGHT};
use super::game::{ClearKind, DEADLY_COLS, attack_for, t_spin_kind};
use super::piece::{PieceKind, Rot};
use super::srs::kicks;

/// One virtual input in a placement plan. Plans implicitly end with a
/// hard drop after the last step, so a plan whose final step is a
/// rotation locks as a T-spin when the corner rule agrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Cw,
    Ccw,
    /// Sonic drop: fall onto the stack without locking (what a human does
    /// by holding max-SDF soft drop before a tuck or spin).
    Drop,
}

/// A concrete decision for the current piece: optionally hold, replay the
/// steps, then hard drop.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub use_hold: bool,
    pub steps: Vec<Step>,
    pub score: f32,
}

/// Highest selectable stage.
pub const MAX_STAGE: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    /// All-round parameters.
    Balanced,
    /// Fast hands, sloppy evaluation, shallow search.
    Rusher,
    /// Slow hands, sharp evaluation, deeper search.
    Thinker,
    /// Chases T-spins whenever the board allows one.
    Spinner,
}

impl Archetype {
    pub fn label(self) -> &'static str {
        match self {
            Archetype::Balanced => "BALANCED",
            Archetype::Rusher => "RUSHER",
            Archetype::Thinker => "THINKER",
            Archetype::Spinner => "SPINNER",
        }
    }

    fn for_stage(stage: u32) -> Archetype {
        match stage % 7 {
            3 => Archetype::Rusher,
            5 => Archetype::Thinker,
            6 => Archetype::Spinner,
            _ => Archetype::Balanced,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AiProfile {
    /// Preview pieces the beam search reads past the current one (0 = greedy).
    pub search_depth: u8,
    /// Candidate lines that survive each beam-search ply.
    pub beam_width: u8,
    /// Full pathfinding movegen: soft-drop tucks and SRS spins. When off
    /// the piece is only ever rotated at the top and hard-dropped.
    pub finesse: bool,
    /// 0..1: how much building and using T-spin slots is worth.
    pub tspin_focus: f32,
    /// May use the hold slot.
    pub uses_hold: bool,
    /// Standard deviation of gaussian-ish noise added to evaluations.
    pub eval_noise: f32,
    /// Seconds between virtual key presses when executing a plan.
    pub action_interval: f32,
    /// Extra thinking pause after a piece spawns.
    pub think_time: f32,
    /// Probability of committing to a knowingly suboptimal placement.
    pub mistake_rate: f32,
    /// 0.0 = pure survival evaluation (classic Dellacherie: any clear is
    /// good). 1.0 = attack-oriented: clears are valued by the garbage they
    /// send, zero-attack singles are penalized and a tetris well is worth
    /// keeping. High stages ramp this up so they stop wasting the stack on
    /// harmless single clears.
    pub attack_focus: f32,
    pub archetype: Archetype,
}

impl AiProfile {
    /// Difficulty ladder: smooth skill curves over stages 1..=30 with
    /// deterministic per-stage jitter and periodic archetypes so
    /// consecutive stages feel like different opponents, not the same one
    /// with a bigger number.
    pub fn for_stage(stage: u32) -> Self {
        Self::for_stage_styled(stage, None)
    }

    /// Ladder profile with an optional archetype override (custom match).
    pub fn for_stage_styled(stage: u32, style: Option<Archetype>) -> Self {
        let stage = stage.clamp(1, MAX_STAGE);
        let t = (stage - 1) as f32 / (MAX_STAGE - 1) as f32;
        let ease = t * t * (3.0 - 2.0 * t); // smoothstep skill ramp

        let mut profile = Self {
            search_depth: match stage {
                0..=9 => 0,
                10..=19 => 1,
                20..=25 => 2,
                _ => 3,
            },
            beam_width: (4.0 + ease * 12.0).round() as u8,
            finesse: stage >= 8,
            tspin_focus: ((stage as f32 - 8.0) / 16.0).clamp(0.0, 1.0) * 0.8,
            uses_hold: stage >= 6,
            eval_noise: 9.0 * (1.0 - ease).powf(1.4),
            action_interval: 0.30 - 0.27 * ease,
            // NB: must stay below LOCK_DELAY (0.5 s) or a piece spawning
            // onto a tall stack locks before the CPU makes a single input.
            think_time: 0.45 - 0.39 * ease,
            mistake_rate: 0.32 * (1.0 - ease).powf(1.2),
            attack_focus: ((stage as f32 - 9.0) / 16.0).clamp(0.0, 1.0),
            archetype: style.unwrap_or_else(|| Archetype::for_stage(stage)),
        };

        // Deterministic jitter (±12%) so the ladder isn't perfectly linear.
        let mut jr = StdRng::seed_from_u64(stage as u64 * 7919 + 13);
        let mut jitter = |v: f32| v * jr.random_range(0.88..1.12);
        profile.eval_noise = jitter(profile.eval_noise);
        profile.action_interval = jitter(profile.action_interval).max(0.03);
        profile.think_time = jitter(profile.think_time).clamp(0.05, 0.45);
        profile.mistake_rate = jitter(profile.mistake_rate).clamp(0.0, 0.5);

        match profile.archetype {
            Archetype::Rusher => {
                profile.action_interval *= 0.55;
                profile.think_time *= 0.7;
                profile.eval_noise *= 1.6;
                profile.mistake_rate = (profile.mistake_rate * 1.3).min(0.5);
                // Rushers spam whatever clears fastest, and don't look far.
                profile.attack_focus *= 0.85;
                profile.search_depth = profile.search_depth.min(1);
            }
            Archetype::Thinker => {
                profile.action_interval *= 1.35;
                profile.think_time = (profile.think_time * 1.4).min(0.45);
                profile.eval_noise *= 0.45;
                profile.mistake_rate *= 0.5;
                profile.attack_focus = (profile.attack_focus + 0.15).min(1.0);
                profile.search_depth = (profile.search_depth + 1).min(3);
                profile.beam_width = profile.beam_width.saturating_add(4);
            }
            Archetype::Spinner => {
                profile.finesse = true;
                profile.tspin_focus = profile.tspin_focus.max(0.7);
                profile.attack_focus = (profile.attack_focus + 0.2).min(1.0);
            }
            Archetype::Balanced => {}
        }

        if !profile.finesse {
            profile.tspin_focus = 0.0;
        }
        // The final stretch plays essentially perfectly.
        if stage >= 26 {
            profile.eval_noise = 0.0;
            profile.mistake_rate = 0.0;
        }
        profile
    }

    pub fn easy() -> Self {
        Self::for_stage(4)
    }

    pub fn normal() -> Self {
        Self::for_stage(15)
    }

    pub fn hard() -> Self {
        Self::for_stage(28)
    }
}

/// Number of distinct rotation states per piece.
fn distinct_rotations(kind: PieceKind) -> &'static [Rot] {
    match kind {
        PieceKind::O => &[Rot::R0],
        PieceKind::I | PieceKind::S | PieceKind::Z => &[Rot::R0, Rot::R1],
        _ => &[Rot::R0, Rot::R1, Rot::R2, Rot::R3],
    }
}

/// A reachable resting spot together with the inputs that get there.
struct Placement {
    piece: ActivePiece,
    /// Kick-table index if the last input was a rotation (T-spin rule).
    last_kick: Option<usize>,
    steps: Vec<Step>,
}

/// x is allowed to roam a little outside the field for wall kicks.
const X_STATES: usize = (BOARD_WIDTH + 4) as usize;

fn state_index(p: &ActivePiece, spin: bool) -> Option<usize> {
    let x = p.x as isize + 2;
    let y = p.y as isize;
    if !(0..X_STATES as isize).contains(&x) || !(0..BOARD_HEIGHT as isize).contains(&y) {
        return None;
    }
    Some(
        ((p.rot.index() * X_STATES + x as usize) * BOARD_HEIGHT as usize + y as usize) * 2
            + spin as usize,
    )
}

/// Every resting position reachable from `start` and a shortest input
/// path to each: BFS over (rotation, x, y) with moves, SRS rotations and
/// sonic drops. Because a rotation is a distinct final input, spins and
/// their kicks fall out of the search for free. States are additionally
/// keyed on "was the last input a rotation" for T pieces only, so a slot
/// reachable both by sliding and by spinning keeps the spin variant.
fn reachable_placements(board: &Board, start: ActivePiece, finesse: bool) -> Vec<Placement> {
    if !board.fits(&start) {
        return Vec::new();
    }
    if !finesse {
        return drop_only_placements(board, &start);
    }

    let mut visited = vec![false; 4 * X_STATES * BOARD_HEIGHT as usize * 2];
    let mut queue: VecDeque<(ActivePiece, Option<usize>, Vec<Step>)> = VecDeque::new();
    let mut seen_final: HashSet<([(i8, i8); 4], u8)> = HashSet::new();
    let mut out = Vec::new();

    if let Some(i) = state_index(&start, false) {
        visited[i] = true;
    }
    queue.push_back((start, None, Vec::new()));

    while let Some((piece, kick, steps)) = queue.pop_front() {
        let grounded = !board.fits(&piece.shifted(0, -1));
        if grounded {
            // Candidate placement; dedup by the cells it fills plus the
            // T-spin classification (BFS order keeps the shortest path).
            let tspin = t_spin_kind(board, &piece, kick);
            let mut cells = piece.board_cells();
            cells.sort_unstable();
            let tag = match tspin {
                None => 0u8,
                Some(ClearKind::TSpinMini) => 1,
                Some(ClearKind::TSpin) => 2,
                Some(ClearKind::Normal) => 3,
            };
            if seen_final.insert((cells, tag)) {
                out.push(Placement {
                    piece,
                    last_kick: kick,
                    steps: steps.clone(),
                });
            }
        }

        for (dx, step) in [(-1, Step::Left), (1, Step::Right)] {
            let cand = piece.shifted(dx, 0);
            if board.fits(&cand) {
                if let Some(i) = state_index(&cand, false) {
                    if !visited[i] {
                        visited[i] = true;
                        let mut s = steps.clone();
                        s.push(step);
                        queue.push_back((cand, None, s));
                    }
                }
            }
        }
        for (cw, step) in [(true, Step::Cw), (false, Step::Ccw)] {
            let to = if cw { piece.rot.cw() } else { piece.rot.ccw() };
            for (i, &(dx, dy)) in kicks(piece.kind, piece.rot, to).iter().enumerate() {
                let cand = piece.rotated(to).shifted(dx, dy);
                if board.fits(&cand) {
                    let spin = piece.kind == PieceKind::T;
                    if let Some(idx) = state_index(&cand, spin) {
                        if !visited[idx] {
                            visited[idx] = true;
                            let mut s = steps.clone();
                            s.push(step);
                            queue.push_back((cand, Some(i), s));
                        }
                    }
                    break;
                }
            }
        }
        if !grounded {
            let mut cand = piece;
            while board.fits(&cand.shifted(0, -1)) {
                cand = cand.shifted(0, -1);
            }
            if let Some(i) = state_index(&cand, false) {
                if !visited[i] {
                    visited[i] = true;
                    let mut s = steps.clone();
                    s.push(Step::Drop);
                    queue.push_back((cand, None, s));
                }
            }
        }
    }
    out
}

/// Low-stage movegen: rotate at the top, shift, hard drop — no spins or
/// tucks, matching how a beginner places pieces.
fn drop_only_placements(board: &Board, start: &ActivePiece) -> Vec<Placement> {
    let mut out = Vec::with_capacity(40);
    for &rot in distinct_rotations(start.kind) {
        let cw_steps = (4 + rot.index() as i8 - start.rot.index() as i8) % 4;
        let rot_steps: &[Step] = match cw_steps {
            1 => &[Step::Cw],
            2 => &[Step::Cw, Step::Cw],
            3 => &[Step::Ccw],
            _ => &[],
        };
        for x in -2..BOARD_WIDTH {
            let mut piece = ActivePiece {
                kind: start.kind,
                rot,
                x,
                y: BOARD_HEIGHT - start.kind.box_size(),
            };
            if !board.fits(&piece) {
                continue;
            }
            while board.fits(&piece.shifted(0, -1)) {
                piece = piece.shifted(0, -1);
            }
            let mut steps = rot_steps.to_vec();
            let dx = x - start.x;
            for _ in 0..dx.abs() {
                steps.push(if dx < 0 { Step::Left } else { Step::Right });
            }
            out.push(Placement {
                piece,
                last_kick: None,
                steps,
            });
        }
    }
    out
}

/// What locking a placement does to the board.
struct Sim {
    board: Board,
    lines: u32,
    tspin: Option<ClearKind>,
    perfect: bool,
    landing_height: f32,
    eroded: f32,
    /// The resulting stack walls off the spawn area (deadly next piece).
    spawn_blocked: bool,
}

fn simulate(board: &Board, piece: &ActivePiece, last_kick: Option<usize>) -> Sim {
    let tspin = t_spin_kind(board, piece, last_kick);
    let cells = piece.board_cells();
    let min_y = cells.iter().map(|c| c.1).min().unwrap_or(0);
    let max_y = cells.iter().map(|c| c.1).max().unwrap_or(0);
    let mut b = board.clone();
    b.lock(piece);
    let cleared_rows = b.clear_full_rows();
    let piece_cells_eroded = cells.iter().filter(|c| cleared_rows.contains(&c.1)).count();
    let spawn_blocked = (3..=6).any(|x| b.cell(x, 20).is_some() || b.cell(x, 21).is_some());
    Sim {
        lines: cleared_rows.len() as u32,
        tspin,
        perfect: b.is_empty(),
        landing_height: (min_y + max_y) as f32 / 2.0,
        eroded: (cleared_rows.len() * piece_cells_eroded) as f32,
        spawn_blocked,
        board: b,
    }
}

/// B2B / combo bookkeeping across search plies, mirroring the game rules.
fn next_ctx(sim: &Sim, b2b: bool, combo: i32) -> (bool, i32) {
    if sim.lines == 0 {
        (b2b, -1)
    } else {
        (sim.lines == 4 || sim.tspin.is_some(), combo + 1)
    }
}

/// Immediate value of locking one placement (line-clear terms only; the
/// board structure is scored separately).
fn move_reward(sim: &Sim, b2b_armed: bool, combo_prev: i32, profile: &AiProfile) -> f32 {
    let mut r = -4.500 * sim.landing_height;
    if sim.lines == 0 {
        return r;
    }
    let kind = sim.tspin.unwrap_or(ClearKind::Normal);
    let difficult = sim.lines == 4 || sim.tspin.is_some();
    let b2b = difficult && b2b_armed;
    let combo = (combo_prev + 1).max(0) as u32;
    let attack = attack_for(kind, sim.lines, b2b, combo, sim.perfect);

    // Survival play rewards any erosion of the stack; attack play pays by
    // garbage sent and dings zero-attack singles.
    let survival = 3.418 * sim.eroded;
    let mut attack_term = attack as f32 * 12.0;
    if sim.lines == 1 && sim.tspin.is_none() {
        attack_term -= 12.0;
    }
    r += survival * (1.0 - profile.attack_focus) + attack_term * profile.attack_focus;

    // Spins are worth chasing beyond their raw attack: they keep B2B
    // alive and only spend three cells of stack per line sent.
    let style = match (kind, sim.lines) {
        (ClearKind::TSpin, 1) => 8.0,
        (ClearKind::TSpin, 2) => 30.0,
        (ClearKind::TSpin, 3) => 50.0,
        _ => 0.0,
    };
    r + style * profile.tspin_focus
}

/// Find a ready T-spin-double notch: a row with one center hole under a
/// row open exactly three wide, roofed by an overhang on one side. This
/// is the shape `reachable_placements` can convert with any T piece.
fn find_tsd_slot(b: &Board) -> Option<(i8, i8)> {
    let max_h = *b.column_heights().iter().max().unwrap_or(&0);
    for by in 0..max_h.min(VISIBLE_HEIGHT - 2) {
        for bx in 0..=BOARD_WIDTH - 3 {
            // Bottom row: full except the T-nose hole at bx+1.
            if b.cell(bx + 1, by).is_some() {
                continue;
            }
            if (0..BOARD_WIDTH).any(|x| x != bx + 1 && b.cell(x, by).is_none()) {
                continue;
            }
            // Middle row: open exactly at bx..=bx+2.
            if (bx..=bx + 2).any(|x| b.cell(x, by + 1).is_some()) {
                continue;
            }
            if (0..BOARD_WIDTH).any(|x| !(bx..=bx + 2).contains(&x) && b.cell(x, by + 1).is_none())
            {
                continue;
            }
            // A roof on exactly one side (the third corner), with the
            // entry column above the nose still open for the T to drop in.
            let left = b.cell(bx, by + 2).is_some();
            let right = b.cell(bx + 2, by + 2).is_some();
            if (left ^ right) && b.cell(bx + 1, by + 2).is_none() {
                return Some((bx, by));
            }
        }
    }
    None
}

/// Structural evaluation of a board (Dellacherie terms plus T-slot
/// awareness). `t_soon`: a T piece is in hold or the visible queue.
fn board_score(b: &Board, profile: &AiProfile, t_soon: bool) -> f32 {
    let mut row_transitions = 0i32;
    for y in 0..BOARD_HEIGHT {
        let mut prev = true; // left border counts as filled
        for x in 0..BOARD_WIDTH {
            let filled = b.cell(x, y).is_some();
            if filled != prev {
                row_transitions += 1;
            }
            prev = filled;
        }
        if !prev {
            row_transitions += 1; // right border
        }
    }

    let mut col_transitions = 0i32;
    let mut holes = 0i32;
    let mut wells = 0i32;
    let mut max_well = 0i32;
    for x in 0..BOARD_WIDTH {
        let mut prev = true; // floor counts as filled
        let mut covered = false;
        for y in 0..BOARD_HEIGHT {
            let filled = b.cell(x, y).is_some();
            if filled != prev {
                col_transitions += 1;
            }
            prev = filled;
        }
        // Holes: empty cells with any filled cell above.
        for y in (0..BOARD_HEIGHT).rev() {
            let filled = b.cell(x, y).is_some();
            if filled {
                covered = true;
            } else if covered {
                holes += 1;
            }
        }
        // Cumulative wells: empty cells whose left and right neighbors are
        // filled (borders count), weighted by depth (1+2+3...).
        let mut depth = 0i32;
        for y in (0..BOARD_HEIGHT).rev() {
            let open = b.cell(x, y).is_none();
            let left = x == 0 || b.cell(x - 1, y).is_some();
            let right = x == BOARD_WIDTH - 1 || b.cell(x + 1, y).is_some();
            if open && left && right {
                depth += 1;
                wells += depth;
                max_well = max_well.max(depth);
            } else {
                depth = 0;
            }
        }
    }

    // Keeping one deep well open is the whole point of tetris play: the
    // cumulative well penalty relaxes as attack focus rises, and a clean
    // board (no holes) earns an explicit bonus for a tetris-ready well.
    let well_weight = 3.386 * (1.0 - 0.55 * profile.attack_focus);
    let well_bonus = if holes == 0 {
        profile.attack_focus * 4.5 * max_well.min(4) as f32
    } else {
        0.0
    };

    let mut score = -3.218 * row_transitions as f32
        - 9.348 * col_transitions as f32
        - 7.899 * holes as f32
        - well_weight * wells as f32
        + well_bonus;

    // A ready TSD notch is money in the bank (it outweighs the one
    // covered cell the hole counter charges it for) — more so when a T
    // is actually on the way.
    if profile.tspin_focus > 0.0 && find_tsd_slot(b).is_some() {
        score += profile.tspin_focus * (16.0 + if t_soon { 14.0 } else { 0.0 });
    }
    score
}

/// A placement that would lock out (fully above the skyline in the
/// deadly center columns) loses instantly and is never a candidate.
fn is_lock_out(piece: &ActivePiece) -> bool {
    let cells = piece.board_cells();
    cells.iter().all(|&(_, y)| y >= VISIBLE_HEIGHT)
        && cells.iter().any(|&(x, _)| DEADLY_COLS.contains(&x))
}

/// The state a piece spawns in on `board` (guideline: drop one row when
/// unobstructed), or None if it would block out.
fn spawn_state(board: &Board, kind: PieceKind) -> Option<ActivePiece> {
    let piece = ActivePiece::spawn(kind);
    if !board.fits(&piece) {
        return None;
    }
    let dropped = piece.shifted(0, -1);
    Some(if board.fits(&dropped) { dropped } else { piece })
}

/// One line of play the beam search is tracking.
#[derive(Clone)]
struct Node {
    board: Board,
    /// Accumulated move rewards along this line.
    acc: f32,
    /// acc + structural score of `board` (the beam's ranking key).
    score: f32,
    b2b: bool,
    combo: i32,
    /// Queue offset: 1 when the root move consumed queue[0] via hold.
    q_off: usize,
    /// Index of the root plan this line descends from.
    root: usize,
}

/// Choose a placement for the current situation.
///
/// `active` is the falling piece as it currently stands; `queue` the
/// visible previews. `incoming` is the number of queued garbage rows —
/// the planner treats them as imminent height and favors clears that
/// cancel them. `b2b_armed` / `combo` mirror the game state so future
/// clears are valued with the right bonuses. Returns None only if no
/// placement fits.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    board: &Board,
    active: ActivePiece,
    hold: Option<PieceKind>,
    queue: &[PieceKind],
    incoming: u32,
    b2b_armed: bool,
    combo: i32,
    profile: &AiProfile,
    rng: &mut StdRng,
) -> Option<Plan> {
    // Danger is judged on the CURRENT board plus queued garbage, so a clear
    // that reduces the stack is not penalized for leaving "low danger".
    let base_height = *board.column_heights().iter().max().unwrap_or(&0) as f32;
    let danger = base_height + incoming.min(8) as f32;
    let horizon = 1 + profile.search_depth as usize;
    let t_soon = hold == Some(PieceKind::T)
        || active.kind == PieceKind::T
        || queue.iter().take(horizon).any(|k| *k == PieceKind::T);

    // Root moves: the current piece, plus the hold alternative. Holding
    // with an empty slot consumes queue[0], so that line reads previews
    // one deeper (q_off).
    let mut starts: Vec<(ActivePiece, bool, usize)> = vec![(active, false, 0)];
    if profile.uses_hold {
        match hold {
            Some(h) if h != active.kind => {
                if let Some(s) = spawn_state(board, h) {
                    starts.push((s, true, 0));
                }
            }
            None => {
                if let Some(&n) = queue.first() {
                    if n != active.kind {
                        if let Some(s) = spawn_state(board, n) {
                            starts.push((s, true, 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let mut roots: Vec<Plan> = Vec::with_capacity(96);
    let mut nodes: Vec<Node> = Vec::with_capacity(96);
    for (start, use_hold, q_off) in starts {
        for pl in reachable_placements(board, start, profile.finesse) {
            if is_lock_out(&pl.piece) {
                continue;
            }
            let sim = simulate(board, &pl.piece, pl.last_kick);
            let mut reward = move_reward(&sim, b2b_armed, combo, profile);
            if sim.spawn_blocked {
                reward -= 10_000.0;
            }
            // Survival instinct: prefer clearing lines when the stack
            // (plus garbage about to rise) is dangerously high, and
            // cancel garbage aggressively.
            if danger >= 12.0 {
                reward += sim.lines as f32 * 12.0;
            }
            if incoming > 0 {
                reward += sim.lines as f32 * (4.0 + 2.0 * incoming.min(8) as f32);
            }
            let (b2b2, combo2) = next_ctx(&sim, b2b_armed, combo);
            let score = reward + board_score(&sim.board, profile, t_soon);
            roots.push(Plan {
                use_hold,
                steps: pl.steps,
                score,
            });
            nodes.push(Node {
                board: sim.board,
                acc: reward,
                score,
                b2b: b2b2,
                combo: combo2,
                q_off,
                root: roots.len() - 1,
            });
        }
    }
    if roots.is_empty() {
        return None;
    }

    // Beam search across the preview queue.
    for ply in 0..profile.search_depth as usize {
        nodes.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
        nodes.truncate((profile.beam_width as usize).max(1));
        let mut next: Vec<Node> = Vec::with_capacity(nodes.len() * 40);
        for node in &nodes {
            let Some(&kind) = queue.get(ply + node.q_off) else {
                next.push(node.clone());
                continue;
            };
            let mut expanded = false;
            if let Some(start) = spawn_state(&node.board, kind) {
                for pl in reachable_placements(&node.board, start, profile.finesse) {
                    if is_lock_out(&pl.piece) {
                        continue;
                    }
                    let sim = simulate(&node.board, &pl.piece, pl.last_kick);
                    let mut reward = move_reward(&sim, node.b2b, node.combo, profile);
                    if sim.spawn_blocked {
                        reward -= 10_000.0;
                    }
                    let (b2b2, combo2) = next_ctx(&sim, node.b2b, node.combo);
                    let acc = node.acc + reward;
                    next.push(Node {
                        score: acc + board_score(&sim.board, profile, t_soon),
                        board: sim.board,
                        acc,
                        b2b: b2b2,
                        combo: combo2,
                        q_off: node.q_off,
                        root: node.root,
                    });
                    expanded = true;
                }
            }
            if !expanded {
                // Dead end (block-out next piece): keep the line but sink it.
                let mut dead = node.clone();
                dead.score -= 10_000.0;
                next.push(dead);
            }
        }
        nodes = next;
    }

    // Best surviving line per root move; pruned roots sink to the bottom.
    let mut best = vec![f32::NEG_INFINITY; roots.len()];
    for node in &nodes {
        best[node.root] = best[node.root].max(node.score);
    }
    for (i, b) in best.iter_mut().enumerate() {
        if !b.is_finite() {
            *b = roots[i].score - 1e6;
        }
        if profile.eval_noise > 0.0 {
            *b += rng.random_range(-profile.eval_noise..profile.eval_noise);
        }
    }
    let mut order: Vec<usize> = (0..roots.len()).collect();
    order.sort_unstable_by(|&a, &b| best[b].total_cmp(&best[a]));

    // Human-like blunder: sometimes commit to a decent-but-not-best spot.
    // Never blunder when the stack (plus pending garbage) is threatening.
    let mut pick = 0;
    if profile.mistake_rate > 0.0 && danger < 12.0 && order.len() > 1 {
        if rng.random::<f32>() < profile.mistake_rate {
            let k = order.len().min(4);
            pick = rng.random_range(1..k.max(2));
        }
    }
    let chosen = order[pick.min(order.len() - 1)];
    let mut plan = roots.swap_remove(chosen);
    plan.score = best[chosen];
    Some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Cell;
    use crate::game::{Game, GameEvent};
    use rand::SeedableRng;

    /// Replay a plan against a real game, exactly like the CPU driver.
    fn exec(game: &mut Game, plan: &Plan) {
        if plan.use_hold {
            game.hold();
        }
        for step in &plan.steps {
            match step {
                Step::Left => {
                    game.move_horizontal(-1);
                }
                Step::Right => {
                    game.move_horizontal(1);
                }
                Step::Cw => {
                    game.rotate(true);
                }
                Step::Ccw => {
                    game.rotate(false);
                }
                Step::Drop => game.sonic_drop(),
            }
        }
        game.hard_drop();
    }

    #[test]
    fn stage_ladder_gets_harder() {
        let s1 = AiProfile::for_stage(1);
        let s15 = AiProfile::for_stage(15);
        let s30 = AiProfile::for_stage(30);
        // Later stages act faster, think faster and blunder less.
        assert!(s1.action_interval > s15.action_interval);
        assert!(s15.action_interval > s30.action_interval);
        assert!(s1.eval_noise > s15.eval_noise);
        assert!(s1.mistake_rate > s15.mistake_rate);
        assert_eq!(s30.eval_noise, 0.0);
        assert_eq!(s30.mistake_rate, 0.0);
        // Search reads deeper and technique unlocks with the stages.
        assert_eq!(s1.search_depth, 0);
        assert!(s15.search_depth >= 1);
        assert!(s30.search_depth >= 3);
        assert!(s30.beam_width > s1.beam_width);
        assert!(!s1.finesse && !s1.uses_hold);
        assert!(s30.finesse && s30.uses_hold);
        assert_eq!(s1.tspin_focus, 0.0);
        assert!(s30.tspin_focus > 0.5);
        // Attack orientation ramps in for the late ladder.
        assert_eq!(s1.attack_focus, 0.0);
        assert!(s15.attack_focus > 0.0 && s15.attack_focus < 1.0);
        assert_eq!(s30.attack_focus, 1.0);
        // Think time never reaches the lock delay (0.5 s).
        for stage in 1..=MAX_STAGE {
            let p = AiProfile::for_stage(stage);
            assert!(
                p.think_time < 0.5,
                "stage {stage} think_time {}",
                p.think_time
            );
            assert!(p.action_interval >= 0.02);
        }
    }

    #[test]
    fn mistakes_never_fire_in_danger() {
        // With a tall stack the profile's mistake rate must be ignored:
        // plan() picks the top-scored placement deterministically.
        let mut board = Board::new();
        for x in 0..BOARD_WIDTH {
            for y in 0..14 {
                if x != 4 {
                    board.set_cell(x, y, Some(Cell::Garbage));
                }
            }
        }
        let mut profile = AiProfile::for_stage(1);
        profile.mistake_rate = 1.0; // always blunder if allowed
        profile.eval_noise = 0.0; // isolate the mistake mechanism
        let start = ActivePiece::spawn(PieceKind::I);
        let mut reference: Option<Vec<Step>> = None;
        for seed in 0..8 {
            let mut rng = StdRng::seed_from_u64(seed);
            let p = plan(&board, start, None, &[], 0, false, -1, &profile, &mut rng).unwrap();
            match &reference {
                None => reference = Some(p.steps.clone()),
                Some(r) => assert_eq!(*r, p.steps, "danger picks must be deterministic"),
            }
        }
    }

    #[test]
    fn finds_a_placement_on_empty_board() {
        let board = Board::new();
        let mut rng = StdRng::seed_from_u64(1);
        let p = plan(
            &board,
            ActivePiece::spawn(PieceKind::T),
            None,
            &[PieceKind::I],
            0,
            false,
            -1,
            &AiProfile::normal(),
            &mut rng,
        );
        assert!(p.is_some());
    }

    /// Deterministic greedy profile with a chosen attack focus.
    fn exact_profile(attack_focus: f32) -> AiProfile {
        AiProfile {
            search_depth: 0,
            beam_width: 8,
            finesse: false,
            tspin_focus: 0.0,
            uses_hold: false,
            eval_noise: 0.0,
            action_interval: 0.05,
            think_time: 0.1,
            mistake_rate: 0.0,
            attack_focus,
            archetype: Archetype::Balanced,
        }
    }

    #[test]
    fn prefers_completing_a_line() {
        // Row 0 filled except a single-column slot at x=9: the AI must
        // produce a clear. (Survival evaluation, attack_focus 0.)
        let mut game = Game::new(1, 1);
        for x in 0..9 {
            game.board.set_cell(x, 0, Some(Cell::Garbage));
        }
        game.active = ActivePiece::spawn(PieceKind::I);
        let mut rng = StdRng::seed_from_u64(1);
        let p = plan(
            &game.board,
            game.active,
            None,
            &[],
            0,
            false,
            -1,
            &exact_profile(0.0),
            &mut rng,
        )
        .unwrap();
        exec(&mut game, &p);
        assert!(game.lines > 0, "AI should clear the ready line, plan={p:?}");
    }

    #[test]
    fn attack_focused_ai_skips_pointless_singles() {
        // Same single-ready setup, but a fully attack-focused profile:
        // a single sends no garbage, so the AI should stack instead and
        // keep the well open for a bigger clear.
        let mut game = Game::new(1, 1);
        for x in 0..9 {
            game.board.set_cell(x, 0, Some(Cell::Garbage));
        }
        game.active = ActivePiece::spawn(PieceKind::I);
        let mut rng = StdRng::seed_from_u64(1);
        let p = plan(
            &game.board,
            game.active,
            None,
            &[],
            0,
            false,
            -1,
            &exact_profile(1.0),
            &mut rng,
        )
        .unwrap();
        exec(&mut game, &p);
        assert_eq!(
            game.lines, 0,
            "attack-focused AI should not spend the well on a single, plan={p:?}"
        );
    }

    #[test]
    fn avoids_creating_holes() {
        // A single column sticking up: the O must not overhang it.
        let mut game = Game::new(1, 1);
        for y in 0..3 {
            game.board.set_cell(0, y, Some(Cell::Garbage));
        }
        game.active = ActivePiece::spawn(PieceKind::O);
        let mut rng = StdRng::seed_from_u64(2);
        let p = plan(
            &game.board,
            game.active,
            None,
            &[],
            0,
            false,
            -1,
            &AiProfile::normal(),
            &mut rng,
        )
        .unwrap();
        exec(&mut game, &p);
        let mut holes = 0;
        for x in 0..BOARD_WIDTH {
            let mut covered = false;
            for y in (0..BOARD_HEIGHT).rev() {
                if game.board.cell(x, y).is_some() {
                    covered = true;
                } else if covered {
                    holes += 1;
                }
            }
        }
        assert_eq!(holes, 0, "plan={p:?}");
    }

    #[test]
    fn finds_and_executes_tspin_double() {
        // TSD slot at box x=3: row 0 full except the nose hole at col 4,
        // row 1 open at cols 3-5, right-side overhang at (5,2). The AI
        // must sonic-drop the T beside the slot and rotate it in.
        let mut game = Game::new(1, 1);
        for x in 0..BOARD_WIDTH {
            if x != 4 {
                game.board.set_cell(x, 0, Some(Cell::Garbage));
            }
            if !(3..=5).contains(&x) {
                game.board.set_cell(x, 1, Some(Cell::Garbage));
            }
        }
        game.board.set_cell(5, 2, Some(Cell::Garbage));
        game.active = ActivePiece::spawn(PieceKind::T);

        let mut profile = AiProfile::for_stage(28);
        profile.eval_noise = 0.0;
        profile.mistake_rate = 0.0;
        profile.uses_hold = false;
        profile.tspin_focus = 1.0;
        let mut rng = StdRng::seed_from_u64(7);
        let p = plan(
            &game.board,
            game.active,
            None,
            &[],
            0,
            false,
            -1,
            &profile,
            &mut rng,
        )
        .unwrap();
        exec(&mut game, &p);
        let clear = game
            .events
            .iter()
            .find_map(|e| match e {
                GameEvent::Cleared(c) => Some(c.clone()),
                _ => None,
            })
            .expect("the T must clear lines, plan={p:?}");
        assert_eq!(clear.kind, ClearKind::TSpin, "plan={p:?}");
        assert_eq!(clear.lines, 2, "plan={p:?}");
    }

    #[test]
    fn finesse_reaches_tucked_cells() {
        // A roof over column 0: cells (0,0)/(0,1) can only be reached by
        // sliding along the floor, never by a plain drop.
        let mut board = Board::new();
        board.set_cell(0, 2, Some(Cell::Garbage));
        let start = ActivePiece::spawn(PieceKind::T);
        let covers = |ps: &[Placement]| {
            ps.iter()
                .any(|p| p.piece.board_cells().iter().any(|&(x, y)| x == 0 && y < 2))
        };
        assert!(
            covers(&reachable_placements(&board, start, true)),
            "finesse movegen must tuck under the roof"
        );
        assert!(
            !covers(&reachable_placements(&board, start, false)),
            "drop-only movegen cannot reach under the roof"
        );
    }

    #[test]
    fn detects_ready_tsd_slot() {
        let mut b = Board::new();
        for x in 0..BOARD_WIDTH {
            if x != 4 {
                b.set_cell(x, 0, Some(Cell::Garbage));
            }
            if !(3..=5).contains(&x) {
                b.set_cell(x, 1, Some(Cell::Garbage));
            }
        }
        b.set_cell(5, 2, Some(Cell::Garbage));
        assert_eq!(find_tsd_slot(&b), Some((3, 0)));
        // Without the overhang it is just a notch, not a T-spin slot.
        b.set_cell(5, 2, None);
        assert_eq!(find_tsd_slot(&b), None);
    }
}
