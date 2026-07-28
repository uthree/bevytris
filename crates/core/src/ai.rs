//! Computer opponent: hard-drop placement search with a Dellacherie-style
//! evaluation function. The Bevy layer executes the chosen plan with
//! human-like timing; this module is pure and synchronous.

use rand::rngs::StdRng;
use rand::Rng;

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
}

impl AiProfile {
    pub fn easy() -> Self {
        Self {
            lookahead: false,
            uses_hold: false,
            eval_noise: 6.0,
            action_interval: 0.22,
            // Must stay below LOCK_DELAY (0.5 s) or a piece spawning onto a
            // tall stack locks before the CPU makes a single input.
            think_time: 0.45,
        }
    }

    pub fn normal() -> Self {
        Self {
            lookahead: false,
            uses_hold: true,
            eval_noise: 1.2,
            action_interval: 0.11,
            think_time: 0.28,
        }
    }

    pub fn hard() -> Self {
        Self {
            lookahead: true,
            uses_hold: true,
            eval_noise: 0.0,
            action_interval: 0.05,
            think_time: 0.12,
        }
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

/// Dellacherie's evaluation, a classic strong single-piece heuristic.
fn evaluate(sim: &Simulated) -> f32 {
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
            } else if !open {
                depth = 0;
            } else {
                depth = 0;
            }
        }
    }

    -4.500 * sim.landing_height + 3.418 * sim.eroded - 3.218 * row_transitions as f32
        - 9.348 * col_transitions as f32
        - 7.899 * holes as f32
        - 3.386 * wells as f32
}

fn best_score_for(board: &Board, kind: PieceKind) -> f32 {
    simulate_placements(board, kind)
        .iter()
        .map(evaluate)
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
    let mut best: Option<Plan> = None;

    // Danger is judged on the CURRENT board plus queued garbage, so a clear
    // that reduces the stack is not penalized for leaving "low danger".
    let base_height = *board.column_heights().iter().max().unwrap_or(&0) as f32;
    let danger = base_height + incoming.min(8) as f32;

    let mut consider = |kind: PieceKind, use_hold: bool, follow: Option<PieceKind>, best: &mut Option<Plan>| {
        for sim in simulate_placements(board, kind) {
            // Never choose a placement that rests entirely above the skyline
            // (instant lock-out).
            if sim.min_y >= VISIBLE_HEIGHT {
                continue;
            }
            let mut score = evaluate(&sim);
            // A placement that walls off the spawn area kills us next piece.
            let spawn_blocked = (3..=6)
                .any(|x| sim.board.cell(x, 20).is_some() || sim.board.cell(x, 21).is_some());
            if spawn_blocked {
                score -= 10_000.0;
            }
            if profile.lookahead {
                if let Some(next_kind) = follow {
                    let follow_score = best_score_for(&sim.board, next_kind);
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
            if best.as_ref().is_none_or(|b| score > b.score) {
                *best = Some(Plan { use_hold, rot: sim.rot, x: sim.x, score });
            }
        }
    };

    consider(current, false, next, &mut best);
    if profile.uses_hold {
        // Holding swaps in either the held piece or (if empty) the next one;
        // in the latter case the true follow-up piece is the second preview.
        match hold {
            Some(h) if h != current => consider(h, true, next, &mut best),
            None => {
                if let Some(n) = next {
                    if n != current {
                        consider(n, true, next2, &mut best);
                    }
                }
            }
            _ => {}
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Cell;
    use rand::SeedableRng;

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

    #[test]
    fn prefers_completing_a_line() {
        // Row 0 filled except a single-column slot at x=9: a vertical I
        // there clears a line; anything else leaves a bumpy mess.
        let mut board = Board::new();
        for x in 0..9 {
            for y in 0..1 {
                board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
        let mut rng = StdRng::seed_from_u64(1);
        let p = plan(&board, PieceKind::I, None, None, None, 0, &AiProfile::hard(), &mut rng).unwrap();
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
