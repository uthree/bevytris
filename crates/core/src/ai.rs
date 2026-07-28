//! Computer opponent: hard-drop placement search with a Dellacherie-style
//! evaluation function. The Bevy layer executes the chosen plan with
//! human-like timing; this module is pure and synchronous.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::board::{ActivePiece, Board, BOARD_HEIGHT, BOARD_WIDTH, VISIBLE_HEIGHT};
use super::piece::{PieceKind, Rot};

/// A concrete placement decision for the current piece.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub use_hold: bool,
    pub rot: Rot,
    /// Target bounding-box x position.
    pub x: i8,
    pub score: f32,
}

/// Highest selectable stage.
pub const MAX_STAGE: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    /// All-round parameters.
    Balanced,
    /// Fast hands, sloppy evaluation.
    Rusher,
    /// Slow hands, sharp evaluation (earlier lookahead).
    Thinker,
}

impl Archetype {
    pub fn label(self) -> &'static str {
        match self {
            Archetype::Balanced => "BALANCED",
            Archetype::Rusher => "RUSHER",
            Archetype::Thinker => "THINKER",
        }
    }

    fn for_stage(stage: u32) -> Archetype {
        match stage % 7 {
            3 => Archetype::Rusher,
            5 => Archetype::Thinker,
            _ => Archetype::Balanced,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AiProfile {
    /// Look at the next piece too (depth 2) instead of greedy depth 1.
    pub lookahead: bool,
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
        let stage = stage.clamp(1, MAX_STAGE);
        let t = (stage - 1) as f32 / (MAX_STAGE - 1) as f32;
        let ease = t * t * (3.0 - 2.0 * t); // smoothstep skill ramp

        let mut profile = Self {
            lookahead: stage >= 14,
            uses_hold: stage >= 6,
            eval_noise: 9.0 * (1.0 - ease).powf(1.4),
            action_interval: 0.30 - 0.27 * ease,
            // NB: must stay below LOCK_DELAY (0.5 s) or a piece spawning
            // onto a tall stack locks before the CPU makes a single input.
            think_time: 0.45 - 0.39 * ease,
            mistake_rate: 0.32 * (1.0 - ease).powf(1.2),
            attack_focus: ((stage as f32 - 9.0) / 16.0).clamp(0.0, 1.0),
            archetype: Archetype::for_stage(stage),
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
                // Rushers spam whatever clears fastest.
                profile.attack_focus *= 0.85;
            }
            Archetype::Thinker => {
                profile.action_interval *= 1.35;
                profile.think_time = (profile.think_time * 1.4).min(0.45);
                profile.eval_noise *= 0.45;
                profile.mistake_rate *= 0.5;
                profile.lookahead = profile.lookahead || stage >= 10;
                profile.attack_focus = (profile.attack_focus + 0.15).min(1.0);
            }
            Archetype::Balanced => {}
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

struct Simulated {
    rot: Rot,
    x: i8,
    board: Board,
    landing_height: f32,
    eroded: f32,
    lines: u32,
    /// Lowest row occupied by the piece where it came to rest.
    min_y: i8,
}

/// All reachable hard-drop placements of `kind` on `board`.
fn simulate_placements(board: &Board, kind: PieceKind) -> Vec<Simulated> {
    let mut out = Vec::with_capacity(40);
    for &rot in distinct_rotations(kind) {
        for x in -2..BOARD_WIDTH {
            let mut piece = ActivePiece { kind, rot, x, y: BOARD_HEIGHT - kind.box_size() };
            if !board.fits(&piece) {
                continue;
            }
            while board.fits(&piece.shifted(0, -1)) {
                piece = piece.shifted(0, -1);
            }
            let cells = piece.board_cells();
            let min_y = cells.iter().map(|c| c.1).min().unwrap_or(0);
            let max_y = cells.iter().map(|c| c.1).max().unwrap_or(0);

            let mut b = board.clone();
            b.lock(&piece);
            let cleared_rows = b.clear_full_rows();
            let piece_cells_eroded = cells
                .iter()
                .filter(|c| cleared_rows.contains(&c.1))
                .count();

            out.push(Simulated {
                rot,
                x,
                board: b,
                landing_height: (min_y + max_y) as f32 / 2.0,
                eroded: (cleared_rows.len() * piece_cells_eroded) as f32,
                lines: cleared_rows.len() as u32,
                min_y,
            });
        }
    }
    out
}

/// Board evaluation. Base is Dellacherie's classic survival heuristic;
/// `attack_focus` blends the line-clear term toward VS efficiency: clears
/// are valued by the garbage they send (single = 0 attack = slightly bad),
/// and the deep-well penalty is relaxed so a tetris well can be kept open.
fn evaluate(sim: &Simulated, attack_focus: f32) -> f32 {
    let b = &sim.board;

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
            } else if !open {
                depth = 0;
            } else {
                depth = 0;
            }
        }
    }

    // Line-clear value: survival play rewards any erosion of the stack;
    // attack play pays by garbage sent and dings zero-attack singles.
    let attack: u32 = match sim.lines {
        2 => 1,
        3 => 2,
        4 => 4,
        _ => 0,
    };
    let survival_clear = 3.418 * sim.eroded;
    let attack_clear =
        attack as f32 * 12.0 - if sim.lines == 1 { 12.0 } else { 0.0 };
    let clear_term = survival_clear * (1.0 - attack_focus) + attack_clear * attack_focus;

    // Keeping one deep well open is the whole point of tetris play: the
    // cumulative well penalty relaxes as attack focus rises, and a clean
    // board (no holes) earns an explicit bonus for a tetris-ready well.
    let well_weight = 3.386 * (1.0 - 0.55 * attack_focus);
    let well_bonus = if holes == 0 {
        attack_focus * 4.5 * max_well.min(4) as f32
    } else {
        0.0
    };

    -4.500 * sim.landing_height + clear_term - 3.218 * row_transitions as f32
        - 9.348 * col_transitions as f32
        - 7.899 * holes as f32
        - well_weight * wells as f32
        + well_bonus
}

fn best_score_for(board: &Board, kind: PieceKind, attack_focus: f32) -> f32 {
    simulate_placements(board, kind)
        .iter()
        .map(|sim| evaluate(sim, attack_focus))
        .fold(f32::NEG_INFINITY, f32::max)
}

/// Choose a placement for the current situation.
///
/// `hold` is the piece currently in the hold slot (if any); `next`/`next2`
/// are the first two preview pieces. `incoming` is the number of queued
/// garbage rows — the planner treats them as imminent height and favors
/// clears that cancel them. Returns None only if no placement fits.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    board: &Board,
    current: PieceKind,
    hold: Option<PieceKind>,
    next: Option<PieceKind>,
    next2: Option<PieceKind>,
    incoming: u32,
    profile: &AiProfile,
    rng: &mut StdRng,
) -> Option<Plan> {
    let mut candidates: Vec<Plan> = Vec::with_capacity(96);

    // Danger is judged on the CURRENT board plus queued garbage, so a clear
    // that reduces the stack is not penalized for leaving "low danger".
    let base_height = *board.column_heights().iter().max().unwrap_or(&0) as f32;
    let danger = base_height + incoming.min(8) as f32;

    let mut consider = |kind: PieceKind, use_hold: bool, follow: Option<PieceKind>, out: &mut Vec<Plan>| {
        for sim in simulate_placements(board, kind) {
            // Never choose a placement that rests entirely above the skyline
            // (instant lock-out).
            if sim.min_y >= VISIBLE_HEIGHT {
                continue;
            }
            let mut score = evaluate(&sim, profile.attack_focus);
            // A placement that walls off the spawn area kills us next piece.
            let spawn_blocked = (3..=6)
                .any(|x| sim.board.cell(x, 20).is_some() || sim.board.cell(x, 21).is_some());
            if spawn_blocked {
                score -= 10_000.0;
            }
            if profile.lookahead {
                if let Some(next_kind) = follow {
                    let follow_score =
                        best_score_for(&sim.board, next_kind, profile.attack_focus);
                    if follow_score.is_finite() {
                        score += 0.6 * follow_score;
                    } else {
                        score -= 1000.0;
                    }
                }
            }
            if profile.eval_noise > 0.0 {
                score += rng.random_range(-profile.eval_noise..profile.eval_noise);
            }
            // Survival instinct: prefer clearing lines when the stack (plus
            // garbage about to rise) is dangerously high, and cancel garbage
            // aggressively.
            if danger >= 12.0 {
                score += sim.lines as f32 * 12.0;
            }
            if incoming > 0 {
                score += sim.lines as f32 * (4.0 + 2.0 * incoming.min(8) as f32);
            }
            out.push(Plan { use_hold, rot: sim.rot, x: sim.x, score });
        }
    };

    consider(current, false, next, &mut candidates);
    if profile.uses_hold {
        // Holding swaps in either the held piece or (if empty) the next one;
        // in the latter case the true follow-up piece is the second preview.
        match hold {
            Some(h) if h != current => consider(h, true, next, &mut candidates),
            None => {
                if let Some(n) = next {
                    if n != current {
                        consider(n, true, next2, &mut candidates);
                    }
                }
            }
            _ => {}
        }
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));

    // Human-like blunder: sometimes commit to a decent-but-not-best spot.
    // Never blunder when the stack (plus pending garbage) is threatening.
    let mut pick = 0;
    if profile.mistake_rate > 0.0 && danger < 12.0 && candidates.len() > 1 {
        if rng.random::<f32>() < profile.mistake_rate {
            let k = candidates.len().min(4);
            pick = rng.random_range(1..k.max(2));
        }
    }
    candidates.get(pick).or_else(|| candidates.first()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Cell;
    use rand::SeedableRng;

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
        assert!(s30.lookahead && s30.uses_hold);
        assert!(!s1.lookahead && !s1.uses_hold);
        // Attack orientation ramps in for the late ladder.
        assert_eq!(s1.attack_focus, 0.0);
        assert!(s15.attack_focus > 0.0 && s15.attack_focus < 1.0);
        assert_eq!(s30.attack_focus, 1.0);
        // Think time never reaches the lock delay (0.5 s).
        for stage in 1..=MAX_STAGE {
            let p = AiProfile::for_stage(stage);
            assert!(p.think_time < 0.5, "stage {stage} think_time {}", p.think_time);
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
        let mut reference: Option<(bool, super::super::piece::Rot, i8)> = None;
        for seed in 0..8 {
            let mut rng = StdRng::seed_from_u64(seed);
            let p = plan(&board, PieceKind::I, None, None, None, 0, &profile, &mut rng).unwrap();
            let key = (p.use_hold, p.rot, p.x);
            match &reference {
                None => reference = Some(key),
                Some(r) => assert_eq!(*r, key, "danger picks must be deterministic"),
            }
        }
    }

    #[test]
    fn finds_a_placement_on_empty_board() {
        let board = Board::new();
        let mut rng = StdRng::seed_from_u64(1);
        let plan = plan(
            &board,
            PieceKind::T,
            None,
            Some(PieceKind::I),
            None,
            0,
            &AiProfile::normal(),
            &mut rng,
        );
        assert!(plan.is_some());
    }

    /// Deterministic profile with a chosen attack focus.
    fn exact_profile(attack_focus: f32) -> AiProfile {
        AiProfile {
            lookahead: false,
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
        // Row 0 filled except a single-column slot at x=9: a vertical I
        // there clears a line; anything else leaves a bumpy mess.
        // (Survival evaluation, attack_focus 0.)
        let mut board = Board::new();
        for x in 0..9 {
            for y in 0..1 {
                board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
        let mut rng = StdRng::seed_from_u64(1);
        let p = plan(&board, PieceKind::I, None, None, None, 0, &exact_profile(0.0), &mut rng).unwrap();
        // Vertical I in box column 2 → box x must be 7 to fill column 9.
        // Horizontal I flat on the floor also plausible; accept either a
        // clear-producing move: simulate and require lines cleared > 0 OR
        // flat placement (x such that it lies flat in empty columns).
        let mut piece = ActivePiece { kind: PieceKind::I, rot: p.rot, x: p.x, y: BOARD_HEIGHT - 4 };
        while board.fits(&piece.shifted(0, -1)) {
            piece = piece.shifted(0, -1);
        }
        let mut b = board.clone();
        b.lock(&piece);
        let cleared = b.clear_full_rows().len();
        assert!(cleared > 0, "AI should clear the ready line, plan={p:?}");
    }

    #[test]
    fn attack_focused_ai_skips_pointless_singles() {
        // Same single-ready setup, but a fully attack-focused profile:
        // a single sends no garbage, so the AI should stack instead and
        // keep the well open for a bigger clear.
        let mut board = Board::new();
        for x in 0..9 {
            board.set_cell(x, 0, Some(Cell::Garbage));
        }
        let mut rng = StdRng::seed_from_u64(1);
        let p = plan(&board, PieceKind::I, None, None, None, 0, &exact_profile(1.0), &mut rng).unwrap();
        let mut piece = ActivePiece { kind: PieceKind::I, rot: p.rot, x: p.x, y: BOARD_HEIGHT - 4 };
        while board.fits(&piece.shifted(0, -1)) {
            piece = piece.shifted(0, -1);
        }
        let mut b = board.clone();
        b.lock(&piece);
        let cleared = b.clear_full_rows().len();
        assert_eq!(cleared, 0, "attack-focused AI should not spend the well on a single, plan={p:?}");
    }

    #[test]
    fn avoids_creating_holes() {
        // A flat floor: any placement is fine, but the AI must not choose
        // one that overhangs the existing single column.
        let mut board = Board::new();
        for y in 0..3 {
            board.set_cell(0, y, Some(Cell::Garbage));
        }
        let mut rng = StdRng::seed_from_u64(2);
        let p = plan(&board, PieceKind::O, None, None, None, 0, &AiProfile::normal(), &mut rng).unwrap();
        // O at box x=-1..0 would stack on the column or next to it; make
        // sure evaluation didn't pick something that creates a hole.
        let mut piece = ActivePiece { kind: PieceKind::O, rot: p.rot, x: p.x, y: BOARD_HEIGHT - 2 };
        while board.fits(&piece.shifted(0, -1)) {
            piece = piece.shifted(0, -1);
        }
        let mut b = board.clone();
        b.lock(&piece);
        b.clear_full_rows();
        let mut holes = 0;
        for x in 0..BOARD_WIDTH {
            let mut covered = false;
            for y in (0..BOARD_HEIGHT).rev() {
                if b.cell(x, y).is_some() {
                    covered = true;
                } else if covered {
                    holes += 1;
                }
            }
        }
        assert_eq!(holes, 0, "plan={p:?}");
    }
}
