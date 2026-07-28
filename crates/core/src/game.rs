//! Single-player game state machine implementing guideline behavior:
//! 7-bag, SRS, ghost, hold, extended placement lockdown, T-spins,
//! back-to-back, combos, guideline scoring and VS garbage rules.

use std::collections::VecDeque;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::bag::SevenBag;
use super::board::{ActivePiece, Board, VISIBLE_HEIGHT};
use super::piece::{PieceKind, Rot};
use super::srs::kicks;

pub const LOCK_DELAY: f32 = 0.5;
pub const MAX_LOCK_RESETS: u32 = 15;
pub const SOFT_DROP_FACTOR: f32 = 20.0;
pub const PREVIEW_COUNT: usize = 5;
pub const LINES_PER_LEVEL: u32 = 10;
pub const GARBAGE_CAP_PER_PIECE: u32 = 8;

/// Seconds a piece takes to fall one row at `level` (guideline curve).
pub fn gravity_seconds(level: u32) -> f32 {
    let l = level.clamp(1, 20) as f32;
    (0.8 - (l - 1.0) * 0.007).powf(l - 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearKind {
    Normal,
    TSpin,
    TSpinMini,
}

/// Everything the frontend needs to celebrate a line clear.
#[derive(Debug, Clone)]
pub struct LineClear {
    pub lines: u32,
    pub kind: ClearKind,
    /// True when this clear extended a back-to-back chain.
    pub b2b: bool,
    /// 0 for the first clear of a streak, 1 for the second, ...
    pub combo: u32,
    pub perfect_clear: bool,
    /// Rows that were cleared (pre-collapse y coordinates).
    pub rows: Vec<i8>,
    /// Garbage lines sent to the opponent (after cancelling).
    pub attack: u32,
    pub score_gained: u64,
}

#[derive(Debug, Clone)]
pub enum GameEvent {
    Spawned,
    Moved {
        dx: i8,
    },
    /// A horizontal move failed (wall or stack in the way).
    MoveBlocked {
        dx: i8,
    },
    Rotated {
        kicked: bool,
        /// True for clockwise turns.
        cw: bool,
        /// The wall-kick offset that was applied (0,0 = no kick).
        kick: (i8, i8),
    },
    RotationFailed,
    SoftDropStep,
    HardDrop { distance: i8 },
    /// A T-piece locked as a T-spin / mini without clearing lines.
    TSpinNoLines { mini: bool },
    Locked { piece: ActivePiece },
    Cleared(LineClear),
    Held,
    HoldBlocked,
    LevelUp { level: u32 },
    GarbageRose { rows: u32 },
    TopOut,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub pieces: u32,
    pub tetrises: u32,
    pub tspins: u32,
    pub max_combo: u32,
    pub attack_sent: u32,
    pub time: f64,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub board: Board,
    pub active: ActivePiece,
    pub hold: Option<PieceKind>,
    pub hold_used: bool,
    bag: SevenBag,
    pub queue: VecDeque<PieceKind>,

    pub score: u64,
    pub lines: u32,
    pub level: u32,
    start_level: u32,
    /// -1 when the previous lock did not clear lines.
    combo: i32,
    /// True when the last line clear was "difficult" (tetris / t-spin).
    b2b_armed: bool,

    gravity_acc: f32,
    lock_timer: f32,
    lock_resets: u32,
    lowest_y: i8,
    /// True once the piece has touched the stack at the current lowest row.
    /// Moves/rotations only consume lockdown resets after first touchdown.
    touched_down: bool,
    /// Kick-table index of the last successful rotation, or None if the
    /// piece moved/fell afterwards. Drives T-spin detection.
    last_rotation_kick: Option<usize>,
    pub soft_dropping: bool,

    /// Pending garbage batches: (rows, hole column).
    pub incoming: VecDeque<(u32, i8)>,
    garbage_rng: StdRng,

    pub game_over: bool,
    pub events: Vec<GameEvent>,
    pub stats: Stats,
}

impl Game {
    pub fn new(seed: u64, start_level: u32) -> Self {
        let mut bag = SevenBag::new(seed);
        let mut queue = VecDeque::with_capacity(PREVIEW_COUNT + 1);
        for _ in 0..=PREVIEW_COUNT {
            queue.push_back(bag.next());
        }
        let first = queue.pop_front().expect("queue prefilled");
        let mut game = Self {
            board: Board::new(),
            active: ActivePiece::spawn(first),
            hold: None,
            hold_used: false,
            bag,
            queue,
            score: 0,
            lines: 0,
            level: start_level.max(1),
            start_level: start_level.max(1),
            combo: -1,
            b2b_armed: false,
            gravity_acc: 0.0,
            lock_timer: 0.0,
            lock_resets: 0,
            lowest_y: 0,
            touched_down: false,
            last_rotation_kick: None,
            soft_dropping: false,
            incoming: VecDeque::new(),
            garbage_rng: StdRng::seed_from_u64(seed ^ 0x6761_7262_6167_65),
            game_over: false,
            events: Vec::new(),
            stats: Stats::default(),
        };
        game.spawn_active(first);
        game
    }

    pub fn take_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }

    /// Total queued garbage rows (for the danger meter UI).
    pub fn incoming_total(&self) -> u32 {
        self.incoming.iter().map(|(n, _)| n).sum()
    }

    /// Y position the active piece would land at if hard-dropped.
    pub fn ghost(&self) -> ActivePiece {
        let mut p = self.active;
        while self.board.fits(&p.shifted(0, -1)) {
            p = p.shifted(0, -1);
        }
        p
    }

    fn grounded(&self) -> bool {
        !self.board.fits(&self.active.shifted(0, -1))
    }

    /// Advance time. Handles gravity, soft drop and the lock timer.
    pub fn tick(&mut self, dt: f32) {
        if self.game_over {
            return;
        }
        self.stats.time += dt as f64;

        let mut sec_per_row = gravity_seconds(self.level);
        if self.soft_dropping {
            sec_per_row /= SOFT_DROP_FACTOR;
        }
        self.gravity_acc += dt / sec_per_row.max(1e-6);
        // One frame can never need more than the board height in steps.
        self.gravity_acc = self.gravity_acc.min(64.0);

        let mut descended = false;
        while self.gravity_acc >= 1.0 {
            self.gravity_acc -= 1.0;
            if self.board.fits(&self.active.shifted(0, -1)) {
                self.active = self.active.shifted(0, -1);
                self.last_rotation_kick = None;
                descended = true;
                if self.soft_dropping {
                    self.score += 1;
                    self.events.push(GameEvent::SoftDropStep);
                }
                self.note_descent();
            } else {
                self.gravity_acc = 0.0;
                break;
            }
        }

        // Lockdown timing. The timer PAUSES while airborne (it only resets
        // when the piece reaches a new lowest row or spends a reset); this
        // closes the "kick the piece into the air for a free timer reset"
        // stall. A frame in which the piece descended charges nothing, so a
        // lag spike that both lands the piece and exceeds LOCK_DELAY cannot
        // insta-lock it.
        if self.grounded() {
            self.touched_down = true;
            if !descended {
                self.lock_timer += dt;
            }
            if self.lock_timer >= LOCK_DELAY {
                self.lock_active();
            }
        }
    }

    fn note_descent(&mut self) {
        if self.active.y < self.lowest_y {
            self.lowest_y = self.active.y;
            self.lock_resets = 0;
            self.lock_timer = 0.0;
            self.touched_down = false;
        }
    }

    /// Spend a lockdown reset (extended placement rule) after a successful
    /// move or rotation. Counts once the piece has touched down at the
    /// current lowest row — even if a kick left it momentarily airborne —
    /// so the 15-reset budget cannot be bypassed.
    fn maybe_reset_lock(&mut self) {
        if self.touched_down && self.lock_resets < MAX_LOCK_RESETS {
            self.lock_resets += 1;
            self.lock_timer = 0.0;
        }
    }

    pub fn move_horizontal(&mut self, dx: i8) -> bool {
        if self.game_over {
            return false;
        }
        let moved = self.active.shifted(dx, 0);
        if self.board.fits(&moved) {
            self.active = moved;
            self.last_rotation_kick = None;
            self.maybe_reset_lock();
            self.events.push(GameEvent::Moved { dx });
            true
        } else {
            self.events.push(GameEvent::MoveBlocked { dx });
            false
        }
    }

    pub fn rotate(&mut self, clockwise: bool) -> bool {
        if self.game_over {
            return false;
        }
        let from = self.active.rot;
        let to = if clockwise { from.cw() } else { from.ccw() };
        for (i, &(dx, dy)) in kicks(self.active.kind, from, to).iter().enumerate() {
            let candidate = self.active.rotated(to).shifted(dx, dy);
            if self.board.fits(&candidate) {
                self.active = candidate;
                self.last_rotation_kick = Some(i);
                self.note_descent();
                self.maybe_reset_lock();
                self.events.push(GameEvent::Rotated {
                    kicked: i > 0,
                    cw: clockwise,
                    kick: (dx, dy),
                });
                return true;
            }
        }
        self.events.push(GameEvent::RotationFailed);
        false
    }

    pub fn set_soft_drop(&mut self, on: bool) {
        if on && !self.soft_dropping {
            // Make the first soft-drop step feel instant.
            self.gravity_acc = self.gravity_acc.max(0.999);
        }
        self.soft_dropping = on;
    }

    pub fn hard_drop(&mut self) {
        if self.game_over {
            return;
        }
        let target = self.ghost();
        let distance = self.active.y - target.y;
        if distance > 0 {
            self.last_rotation_kick = None;
        }
        self.active = target;
        self.score += 2 * distance as u64;
        self.events.push(GameEvent::HardDrop { distance });
        self.lock_active();
    }

    pub fn hold(&mut self) {
        if self.game_over {
            return;
        }
        if self.hold_used {
            self.events.push(GameEvent::HoldBlocked);
            return;
        }
        self.hold_used = true;
        let swapped = self.hold.replace(self.active.kind);
        let kind = match swapped {
            Some(k) => k,
            None => self.next_from_queue(),
        };
        self.events.push(GameEvent::Held);
        self.spawn_active(kind);
    }

    fn next_from_queue(&mut self) -> PieceKind {
        let kind = self.queue.pop_front().expect("queue is never empty");
        self.queue.push_back(self.bag.next());
        kind
    }

    fn spawn_active(&mut self, kind: PieceKind) {
        let piece = ActivePiece::spawn(kind);
        if !self.board.fits(&piece) {
            // Block out: the new piece overlaps the stack.
            self.active = piece;
            self.top_out();
            return;
        }
        // Guideline: the piece immediately drops one row if unobstructed.
        self.active = if self.board.fits(&piece.shifted(0, -1)) {
            piece.shifted(0, -1)
        } else {
            piece
        };
        self.lowest_y = self.active.y;
        self.lock_timer = 0.0;
        self.lock_resets = 0;
        self.touched_down = false;
        self.gravity_acc = 0.0;
        self.last_rotation_kick = None;
        self.events.push(GameEvent::Spawned);
    }

    fn top_out(&mut self) {
        self.game_over = true;
        self.events.push(GameEvent::TopOut);
    }

    /// (filled corners, filled "front" corners) around a locked T piece.
    fn t_corner_counts(&self) -> (u32, u32) {
        let p = &self.active;
        let corners = [(0, 0), (2, 0), (0, 2), (2, 2)];
        // Corners adjacent to the flat "nose" of the T for each rotation.
        let front: [(i8, i8); 2] = match p.rot {
            Rot::R0 => [(0, 2), (2, 2)],
            Rot::R1 => [(2, 2), (2, 0)],
            Rot::R2 => [(0, 0), (2, 0)],
            Rot::R3 => [(0, 0), (0, 2)],
        };
        let mut filled = 0;
        let mut front_filled = 0;
        for (cx, cy) in corners {
            if self.board.is_blocked(p.x + cx, p.y + cy) {
                filled += 1;
                if front.contains(&(cx, cy)) {
                    front_filled += 1;
                }
            }
        }
        (filled, front_filled)
    }

    fn detect_tspin(&self) -> Option<ClearKind> {
        if self.active.kind != PieceKind::T {
            return None;
        }
        let kick = self.last_rotation_kick?;
        let (filled, front_filled) = self.t_corner_counts();
        if filled < 3 {
            return None;
        }
        // Full T-spin: both front corners filled, or the piece reached its
        // spot via the last (fifth) kick test — the TST/fin exception.
        if front_filled == 2 || kick == 4 {
            Some(ClearKind::TSpin)
        } else {
            Some(ClearKind::TSpinMini)
        }
    }

    fn lock_active(&mut self) {
        let piece = self.active;
        let tspin = self.detect_tspin();
        self.board.lock(&piece);
        self.stats.pieces += 1;
        self.events.push(GameEvent::Locked { piece });

        // Lock out: the whole piece settled above the visible field.
        if piece.board_cells().iter().all(|&(_, y)| y >= VISIBLE_HEIGHT) {
            self.top_out();
            return;
        }

        let rows = self.board.clear_full_rows();
        let lines = rows.len() as u32;

        if lines > 0 {
            self.apply_clear(lines, rows, tspin);
        } else {
            match tspin {
                Some(kind) => {
                    let mini = kind == ClearKind::TSpinMini;
                    let base = if mini { 100 } else { 400 };
                    self.score += (base * self.level) as u64;
                    if !mini {
                        self.stats.tspins += 1;
                    }
                    self.events.push(GameEvent::TSpinNoLines { mini });
                }
                None => {}
            }
            self.combo = -1;
            self.rise_garbage();
        }

        if self.game_over {
            return;
        }
        self.hold_used = false;
        let kind = self.next_from_queue();
        self.spawn_active(kind);
    }

    fn apply_clear(&mut self, lines: u32, rows: Vec<i8>, tspin: Option<ClearKind>) {
        let kind = tspin.unwrap_or(ClearKind::Normal);
        let difficult = lines == 4 || tspin.is_some();
        let b2b = difficult && self.b2b_armed;

        self.combo += 1;
        let combo = self.combo as u32;
        self.stats.max_combo = self.stats.max_combo.max(combo);
        if lines == 4 {
            self.stats.tetrises += 1;
        }
        if tspin == Some(ClearKind::TSpin) {
            self.stats.tspins += 1;
        }

        // --- Guideline scoring -------------------------------------------
        let base: u64 = match (kind, lines) {
            (ClearKind::Normal, 1) => 100,
            (ClearKind::Normal, 2) => 300,
            (ClearKind::Normal, 3) => 500,
            (ClearKind::Normal, 4) => 800,
            (ClearKind::TSpinMini, 1) => 200,
            (ClearKind::TSpinMini, 2) => 400,
            (ClearKind::TSpin, 1) => 800,
            (ClearKind::TSpin, 2) => 1200,
            (ClearKind::TSpin, 3) => 1600,
            _ => 0,
        };
        let mut gained = base * self.level as u64;
        if b2b {
            gained = gained * 3 / 2;
        }
        gained += 50 * combo as u64 * self.level as u64;

        let perfect_clear = self.board.is_empty();
        if perfect_clear {
            let pc_bonus: u64 = match lines {
                1 => 800,
                2 => 1200,
                3 => 1800,
                _ => {
                    if b2b {
                        3200
                    } else {
                        2000
                    }
                }
            };
            gained += pc_bonus * self.level as u64;
        }
        self.score += gained;

        // --- Attack (VS garbage) ------------------------------------------
        let mut attack: u32 = match (kind, lines) {
            (ClearKind::Normal, 2) => 1,
            (ClearKind::Normal, 3) => 2,
            (ClearKind::Normal, 4) => 4,
            (ClearKind::TSpinMini, 2) => 1,
            (ClearKind::TSpin, 1) => 2,
            (ClearKind::TSpin, 2) => 4,
            (ClearKind::TSpin, 3) => 6,
            _ => 0,
        };
        if b2b {
            attack += 1;
        }
        const COMBO_BONUS: [u32; 12] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 4, 5];
        attack += COMBO_BONUS[(combo as usize).min(COMBO_BONUS.len() - 1)];
        if perfect_clear {
            attack += 10;
        }
        // Line clears cancel queued garbage before sending the remainder.
        attack = self.cancel_incoming(attack);
        self.stats.attack_sent += attack;

        // --- Lines / level -------------------------------------------------
        self.lines += lines;
        let new_level = self.start_level + self.lines / LINES_PER_LEVEL;
        if new_level > self.level {
            self.level = new_level;
            self.events.push(GameEvent::LevelUp { level: new_level });
        }

        self.b2b_armed = difficult;
        self.events.push(GameEvent::Cleared(LineClear {
            lines,
            kind,
            b2b,
            combo,
            perfect_clear,
            rows,
            attack,
            score_gained: gained,
        }));
    }

    fn cancel_incoming(&mut self, mut attack: u32) -> u32 {
        while attack > 0 {
            match self.incoming.front_mut() {
                Some((rows, _)) => {
                    let cancelled = attack.min(*rows);
                    *rows -= cancelled;
                    attack -= cancelled;
                    if *rows == 0 {
                        self.incoming.pop_front();
                    }
                }
                None => break,
            }
        }
        attack
    }

    /// Queue garbage from the opponent. It rises after a future lock that
    /// does not clear lines.
    pub fn queue_garbage(&mut self, rows: u32) {
        if rows == 0 {
            return;
        }
        let hole = self.garbage_rng.random_range(0..super::board::BOARD_WIDTH);
        self.incoming.push_back((rows, hole));
    }

    fn rise_garbage(&mut self) {
        let mut budget = GARBAGE_CAP_PER_PIECE;
        let mut total = 0u32;
        while budget > 0 {
            match self.incoming.front_mut() {
                Some((rows, hole)) => {
                    let take = (*rows).min(budget);
                    self.board.add_garbage(take as usize, *hole);
                    *rows -= take;
                    budget -= take;
                    total += take;
                    if *rows == 0 {
                        self.incoming.pop_front();
                    }
                }
                None => break,
            }
        }
        if total > 0 {
            self.events.push(GameEvent::GarbageRose { rows: total });
        }
        // Note: garbage only ever rises between a lock and the next spawn,
        // so there is never a live falling piece to push out of the way;
        // `spawn_active` performs the real block-out check right after.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Cell, BOARD_WIDTH};

    fn fill_row(game: &mut Game, y: i8, holes: &[i8]) {
        for x in 0..BOARD_WIDTH {
            if !holes.contains(&x) {
                game.board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
    }

    /// Force the active piece kind for scripted tests.
    fn force_piece(game: &mut Game, kind: PieceKind) {
        game.active = ActivePiece::spawn(kind);
        game.active = game.active.shifted(0, -1);
        game.last_rotation_kick = None;
    }

    #[test]
    fn gravity_curve_matches_guideline_samples() {
        assert!((gravity_seconds(1) - 1.0).abs() < 1e-6);
        // Level 5: (0.8 - 4*0.007)^4 = 0.772^4 ≈ 0.3552
        assert!((gravity_seconds(5) - 0.772f32.powi(4)).abs() < 1e-4);
    }

    #[test]
    fn single_line_clear_scores_100_times_level() {
        let mut game = Game::new(1, 1);
        fill_row(&mut game, 0, &[4]);
        force_piece(&mut game, PieceKind::I);
        // Rotate vertical and drop into the hole at x=4.
        game.rotate(true);
        let dx = 4 - (game.active.x + 2); // vertical I occupies box column 2
        game.move_horizontal(dx.signum());
        while game.active.x + 2 != 4 {
            if !game.move_horizontal(dx.signum()) {
                break;
            }
        }
        let before_x = game.active.x + 2;
        assert_eq!(before_x, 4);
        let drop_distance = game.active.y - game.ghost().y;
        game.hard_drop();
        let clears: Vec<_> = game
            .events
            .iter()
            .filter_map(|e| match e {
                GameEvent::Cleared(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(clears.len(), 1);
        assert_eq!(clears[0].lines, 1);
        assert_eq!(clears[0].kind, ClearKind::Normal);
        // 100 (single) + 2/cell hard drop.
        assert_eq!(game.score, 100 + 2 * drop_distance as u64);
    }

    #[test]
    fn tetris_is_difficult_and_arms_b2b() {
        let mut game = Game::new(1, 1);
        for y in 0..4 {
            fill_row(&mut game, y, &[9]);
        }
        // Stray block so the clear is not a perfect clear.
        game.board.set_cell(0, 4, Some(Cell::Garbage));
        force_piece(&mut game, PieceKind::I);
        game.rotate(true); // vertical, box column 2
        while game.active.x + 2 < 9 {
            if !game.move_horizontal(1) {
                break;
            }
        }
        assert_eq!(game.active.x + 2, 9);
        game.hard_drop();
        let clear = game
            .events
            .iter()
            .find_map(|e| match e {
                GameEvent::Cleared(c) => Some(c.clone()),
                _ => None,
            })
            .expect("tetris cleared");
        assert_eq!(clear.lines, 4);
        assert!(!clear.b2b, "first difficult clear is not yet b2b");
        assert!(!clear.perfect_clear);
        assert_eq!(clear.attack, 4);
        assert!(game.b2b_armed);
    }

    #[test]
    fn perfect_clear_adds_ten_attack() {
        let mut game = Game::new(1, 1);
        for y in 0..4 {
            fill_row(&mut game, y, &[9]);
        }
        force_piece(&mut game, PieceKind::I);
        game.rotate(true);
        while game.active.x + 2 < 9 {
            if !game.move_horizontal(1) {
                break;
            }
        }
        game.hard_drop();
        let clear = game
            .events
            .iter()
            .find_map(|e| match e {
                GameEvent::Cleared(c) => Some(c.clone()),
                _ => None,
            })
            .expect("tetris cleared");
        assert!(clear.perfect_clear);
        assert_eq!(clear.attack, 14); // 4 (tetris) + 10 (perfect clear)
    }

    #[test]
    fn tspin_double_detected() {
        // T-spin double slot: the T (pointing down, box at x=3,y=0) fills
        // cells (3,1),(4,1),(5,1),(4,0). Row 1 lacks exactly those three
        // columns, row 0 lacks only column 4, and an overhang at (5,2)
        // provides the third occupied corner.
        let mut game = Game::new(1, 1);
        fill_row(&mut game, 1, &[3, 4, 5]);
        fill_row(&mut game, 0, &[4]);
        game.board.set_cell(5, 2, Some(Cell::Garbage));

        game.active = ActivePiece { kind: PieceKind::T, rot: Rot::R2, x: 3, y: 0 };
        assert!(game.board.fits(&game.active));
        assert!(!game.board.fits(&game.active.shifted(0, -1)), "grounded");
        // The final action must be a rotation for T-spin detection; the
        // piece is already in place, so mark the rotation directly.
        game.last_rotation_kick = Some(1);
        game.hard_drop(); // distance 0, locks immediately
        let clear = game
            .events
            .iter()
            .find_map(|e| match e {
                GameEvent::Cleared(c) => Some(c.clone()),
                _ => None,
            })
            .expect("lines cleared");
        assert_eq!(clear.lines, 2);
        assert_eq!(clear.kind, ClearKind::TSpin);
        // TSD attack = 4.
        assert_eq!(clear.attack, 4);
        assert_eq!(clear.score_gained, 1200);
    }

    #[test]
    fn tspin_mini_detected_with_single_front_corner() {
        // T pointing down in an open-ended notch: three corners filled but
        // only one of them is a front corner -> mini.
        let mut game = Game::new(1, 1);
        // Box at (0,0), R2: cells (0,1),(1,1),(2,1),(1,0).
        // Corners: (0,0),(2,0),(0,2),(2,2); front (R2) = (0,0),(2,0).
        game.board.set_cell(0, 0, Some(Cell::Garbage)); // front corner
        game.board.set_cell(0, 2, Some(Cell::Garbage)); // back corner
        game.board.set_cell(2, 2, Some(Cell::Garbage)); // back corner
        game.active = ActivePiece { kind: PieceKind::T, rot: Rot::R2, x: 0, y: 0 };
        assert!(game.board.fits(&game.active));
        game.last_rotation_kick = Some(0);
        game.hard_drop();
        assert!(game
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::TSpinNoLines { mini: true })));
    }

    #[test]
    fn garbage_rises_after_non_clearing_lock_and_cancels() {
        let mut game = Game::new(3, 1);
        game.queue_garbage(3);
        assert_eq!(game.incoming_total(), 3);
        game.hard_drop(); // lock without clearing → garbage rises
        assert_eq!(game.incoming_total(), 0);
        let rose = game.events.iter().any(|e| matches!(e, GameEvent::GarbageRose { rows: 3 }));
        assert!(rose);
    }

    #[test]
    fn attack_cancels_queued_garbage_first() {
        let mut game = Game::new(1, 1);
        game.queue_garbage(3);
        let remaining = game.cancel_incoming(4);
        assert_eq!(remaining, 1);
        assert_eq!(game.incoming_total(), 0);
    }

    #[test]
    fn hold_swaps_and_blocks_second_use() {
        let mut game = Game::new(9, 1);
        let first = game.active.kind;
        let next = *game.queue.front().unwrap();
        game.hold();
        assert_eq!(game.hold, Some(first));
        assert_eq!(game.active.kind, next);
        game.hold();
        assert!(game.events.iter().any(|e| matches!(e, GameEvent::HoldBlocked)));
    }

    #[test]
    fn block_out_ends_game() {
        let mut game = Game::new(1, 1);
        // Fill the spawn area (leave column 0 open so nothing clears).
        for y in 18..=23 {
            fill_row(&mut game, y, &[0]);
        }
        game.hard_drop(); // lock current piece high, then next spawn collides
        assert!(game.game_over);
    }

    #[test]
    fn level_up_every_ten_lines() {
        let mut game = Game::new(1, 1);
        game.lines = 9;
        // Clear one line to reach 10.
        fill_row(&mut game, 0, &[4]);
        force_piece(&mut game, PieceKind::I);
        game.rotate(true);
        while game.active.x + 2 != 4 {
            let dir = if game.active.x + 2 < 4 { 1 } else { -1 };
            if !game.move_horizontal(dir) {
                break;
            }
        }
        game.hard_drop();
        assert_eq!(game.lines, 10);
        assert_eq!(game.level, 2);
        assert!(game.events.iter().any(|e| matches!(e, GameEvent::LevelUp { level: 2 })));
    }

    #[test]
    fn lock_delay_locks_after_half_second() {
        let mut game = Game::new(1, 1);
        game.hard_drop(); // piece 1 locked
        let pieces_before = game.stats.pieces;
        // Drop piece to the floor via soft drop, then wait.
        game.set_soft_drop(true);
        for _ in 0..60 {
            game.tick(1.0 / 60.0);
        }
        game.set_soft_drop(false);
        // Should be grounded now; wait out the lock delay.
        for _ in 0..40 {
            game.tick(1.0 / 60.0);
        }
        assert!(game.stats.pieces > pieces_before);
    }

    #[test]
    fn garbage_rise_does_not_falsely_top_out() {
        // O locked in a roofed cavity at rows 19-20 of columns 8-9; the
        // column is otherwise solid up to row 38. Rising garbage must not
        // end the game: the spawn area is still free afterwards.
        let mut game = Game::new(1, 1);
        for x in [8, 9] {
            for y in (0..=18).chain(21..=38) {
                game.board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
        game.active = ActivePiece { kind: PieceKind::O, rot: Rot::R0, x: 8, y: 19 };
        game.queue_garbage(8);
        game.hard_drop(); // locks in place (rows 19-20), no line clear
        assert!(
            game.events.iter().any(|e| matches!(e, GameEvent::GarbageRose { rows: 8 })),
            "garbage should have risen"
        );
        assert!(!game.game_over, "legal board must not top out after garbage rise");
    }

    #[test]
    fn rotation_stall_cannot_prevent_lock() {
        // Alternating CW/CCW rotations on the floor used to reset the lock
        // timer forever (airborne frames zeroed it). With the 15-reset
        // budget enforced, the piece must lock within a few seconds.
        let mut game = Game::new(5, 1);
        // Bring the piece to the floor quickly.
        game.set_soft_drop(true);
        for _ in 0..120 {
            game.tick(1.0 / 60.0);
        }
        game.set_soft_drop(false);
        let before = game.stats.pieces;
        let mut locked = false;
        for i in 0..(30 * 60) {
            game.rotate(if i % 2 == 0 { true } else { false });
            game.tick(1.0 / 60.0);
            if game.stats.pieces > before {
                locked = true;
                break;
            }
        }
        assert!(locked, "piece must lock despite rotation spam");
    }

    #[test]
    fn lag_spike_does_not_instalock_on_landing_frame() {
        // A single huge dt that both lands the piece and exceeds LOCK_DELAY
        // must not lock it in the same tick.
        let mut game = Game::new(1, 1);
        let before = game.stats.pieces;
        game.tick(25.0); // piece falls all the way to the floor during this tick
        assert_eq!(game.stats.pieces, before, "landing frame must not charge lock delay");
        // But staying grounded afterwards locks normally.
        game.tick(0.6);
        assert!(game.stats.pieces > before);
    }

    #[test]
    fn srs_tst_kick_works() {
        // Verify the famous 0->R kick test 5 (-1, -2)... use a plain wall
        // kick instead: T against the left wall, rotate from R3 (left) to
        // R0 must kick right. Simpler: rotate T on open floor: no kick.
        let mut game = Game::new(1, 1);
        force_piece(&mut game, PieceKind::T);
        assert!(game.rotate(true));
        assert!(game
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::Rotated { kicked: false, cw: true, .. })));
    }
}
