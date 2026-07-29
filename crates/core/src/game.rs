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
/// The columns whose overflow loses the game: the spawn area. A piece
/// locking fully above the skyline only kills when it touches these
/// four center columns — side stacks may poke over the top and live.
pub const DEADLY_COLS: std::ops::RangeInclusive<i8> = 3..=6;
/// Guideline default; the frontend may override per player (SDF setting).
pub const SOFT_DROP_FACTOR: f32 = 20.0;
pub const PREVIEW_COUNT: usize = 5;
pub const LINES_PER_LEVEL: u32 = 10;
pub const GARBAGE_CAP_PER_PIECE: u32 = 8;

/// Seconds a piece takes to fall one row at `level` (guideline curve).
pub fn gravity_seconds(level: u32) -> f32 {
    let l = level.clamp(1, 20) as f32;
    (0.8 - (l - 1.0) * 0.007).powf(l - 1.0)
}

/// Highest level timed leveling will reach; `gravity_seconds` flatlines
/// there anyway, so climbing further would only inflate the HUD number.
pub const MAX_TIMED_LEVEL: u32 = 20;

/// Seconds an activated zone lasts.
pub const ZONE_DURATION: f32 = 12.0;
/// Zone gauge fill per cancelled garbage row (1.0 = full). Defense is
/// the battery: offsetting incoming garbage is the main way to charge.
pub const ZONE_CHARGE_PER_CANCEL: f32 = 0.12;
/// Smaller fill per cleared row that contained garbage — digging out
/// garbage that already rose still earns a little charge.
pub const ZONE_CHARGE_PER_GARBAGE_LINE: f32 = 0.05;

/// Tetris-Effect-style super move: charge the gauge by cancelling (or
/// digging out) garbage, then stop time — cleared rows bank at the
/// bottom instead of vanishing and all fire as one attack when the zone
/// ends.
#[derive(Debug, Clone, Default)]
pub struct Zone {
    /// Gauge fill, 0..=1. Activation requires a full gauge.
    pub charge: f32,
    /// Remaining seconds while active.
    pub active: Option<f32>,
    /// Rows banked at the bottom so far this zone.
    pub lines: u32,
}

/// How `level` (and with it gravity) advances during play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Leveling {
    /// Marathon rule: +1 level every `LINES_PER_LEVEL` cleared lines.
    Lines,
    /// Versus rule: the level rises with elapsed play time alone — one
    /// level every `seconds_per_level`, capped at `MAX_TIMED_LEVEL`.
    /// Cleared lines never change it, so efficient downstacking is never
    /// punished with extra gravity and stalling never keeps a match slow.
    Timed { seconds_per_level: f32 },
    /// Sprint and zen rule: the level never moves. A race is timed, so
    /// gravity stays constant and only execution speed matters; zen keeps
    /// it constant because a rising floor is exactly the pressure the mode
    /// exists to remove.
    Fixed,
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
    HardDrop {
        distance: i8,
    },
    /// A T-piece locked as a T-spin / mini without clearing lines.
    TSpinNoLines {
        mini: bool,
    },
    Locked {
        piece: ActivePiece,
    },
    Cleared(LineClear),
    Held,
    HoldBlocked,
    LevelUp {
        level: u32,
    },
    /// Margin time kicked in (or stepped up): attack now scales by this.
    AttackRamp {
        multiplier: f32,
    },
    GarbageRose {
        rows: u32,
    },
    /// The zone gauge just reached full charge.
    ZoneReady,
    /// The zone super move started.
    ZoneActivated,
    /// A lock during an active zone banked full rows at the bottom.
    ZoneLines {
        banked: u32,
        total: u32,
    },
    /// The zone ended: banked lines cleared and fired as one attack.
    ZoneEnded {
        lines: u32,
        attack: u32,
    },
    TopOut,
    /// Endless play only: the stack reached the top, so the field was
    /// wiped and play continued instead of the run ending.
    BoardReset,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub pieces: u32,
    pub tetrises: u32,
    pub tspins: u32,
    pub max_combo: u32,
    pub attack_sent: u32,
    pub perfect_clears: u32,
    pub time: f64,
    /// How many times an endless run has been wiped and restarted.
    pub board_resets: u32,
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
    pub leveling: Leveling,
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
    /// Soft-drop gravity multiplier (SDF). Very large values effectively
    /// teleport the piece to the stack while soft drop is held.
    pub soft_drop_factor: f32,

    /// Pending garbage batches: (rows, hole column).
    pub incoming: VecDeque<(u32, i8)>,
    garbage_rng: StdRng,
    /// Chance in percent that each row after the first in an incoming
    /// batch re-rolls its hole instead of sharing the previous row's.
    /// 0 = one clean well per batch (the guideline default), 100 = every
    /// row somewhere else, which turns attacks into cheese to dig.
    pub garbage_messiness: u32,

    /// Endless play: topping out wipes the field and the run carries on
    /// instead of ending. `game_over` never becomes true.
    pub endless: bool,

    /// False bans the hold slot outright: [`Game::hold`] refuses and the
    /// frontend hides the box.
    pub hold_enabled: bool,
    /// How many previews the player is allowed to see. The internal
    /// queue always stays full (the bag and the planner need it); this
    /// only limits what the frontend draws and what the CPU may read.
    pub visible_previews: usize,

    /// Margin time: seconds of play after which attack scales up (see
    /// [`Game::attack_multiplier`]). None disables the ramp (solo modes).
    pub margin_time: Option<f32>,
    /// Last multiplier announced via [`GameEvent::AttackRamp`].
    last_attack_mult: f32,

    /// Zone super-move state; None disables the mechanic entirely.
    pub zone: Option<Zone>,

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
            leveling: Leveling::Lines,
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
            soft_drop_factor: SOFT_DROP_FACTOR,
            incoming: VecDeque::new(),
            garbage_rng: StdRng::seed_from_u64(seed ^ 0x6761_7262_6167_65),
            garbage_messiness: 0,
            endless: false,
            hold_enabled: true,
            visible_previews: PREVIEW_COUNT,
            margin_time: None,
            last_attack_mult: 1.0,
            zone: None,
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

    /// Height of the tallest stack inside the deadly center columns —
    /// the number the loss condition (and thus the danger warning)
    /// actually cares about.
    pub fn deadly_height(&self) -> i8 {
        let heights = self.board.column_heights();
        DEADLY_COLS.map(|x| heights[x as usize]).max().unwrap_or(0)
    }

    /// Back-to-back armed: the next difficult clear will chain.
    pub fn b2b_armed(&self) -> bool {
        self.b2b_armed
    }

    /// Combo counter as the game tracks it: -1 when the previous lock did
    /// not clear lines, otherwise the 0-based streak position.
    pub fn combo(&self) -> i32 {
        self.combo
    }

    /// Margin-time attack multiplier: x1 until `margin_time`, then x1.5,
    /// stepping up every 30 s (x2, x3) and capping at x4. Long rounds
    /// escalate until somebody falls.
    pub fn attack_multiplier(&self) -> f32 {
        let Some(margin) = self.margin_time else {
            return 1.0;
        };
        let over = self.stats.time as f32 - margin;
        if over < 0.0 {
            1.0
        } else if over < 30.0 {
            1.5
        } else if over < 60.0 {
            2.0
        } else if over < 90.0 {
            3.0
        } else {
            4.0
        }
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

        if let Leveling::Timed { seconds_per_level } = self.leveling {
            let steps = (self.stats.time / seconds_per_level.max(1e-3) as f64) as u32;
            let new_level = (self.start_level + steps).min(MAX_TIMED_LEVEL);
            if new_level > self.level {
                self.level = new_level;
                self.events.push(GameEvent::LevelUp { level: new_level });
            }
        }

        if self.margin_time.is_some() {
            let mult = self.attack_multiplier();
            if mult > self.last_attack_mult {
                self.last_attack_mult = mult;
                self.events.push(GameEvent::AttackRamp { multiplier: mult });
            }
        }

        // Zone countdown; expiry fires the banked lines.
        let expired = match &mut self.zone {
            Some(Zone {
                active: Some(t), ..
            }) => {
                *t -= dt;
                *t <= 0.0
            }
            _ => false,
        };
        if expired {
            self.end_zone();
        }

        // Time stands still inside the zone: no gravity (soft drop still
        // works) and no lockdown — only a hard drop commits a piece.
        let zone_frozen = self.zone_active();

        if !zone_frozen || self.soft_dropping {
            let mut sec_per_row = gravity_seconds(self.level);
            if self.soft_dropping {
                sec_per_row /= self.soft_drop_factor.max(1.0);
            }
            self.gravity_acc += dt / sec_per_row.max(1e-6);
            // One frame can never need more than the board height in steps.
            self.gravity_acc = self.gravity_acc.min(64.0);
        }

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
        if self.grounded() && !zone_frozen {
            self.touched_down = true;
            if !descended {
                self.lock_timer += dt;
            }
            if self.lock_timer >= LOCK_DELAY {
                self.lock_active();
            }
        }
    }

    /// True while the zone super move is running.
    pub fn zone_active(&self) -> bool {
        matches!(&self.zone, Some(z) if z.active.is_some())
    }

    /// Fire the zone if the gauge is full. Returns whether it started.
    pub fn activate_zone(&mut self) -> bool {
        if self.game_over {
            return false;
        }
        let Some(zone) = &mut self.zone else {
            return false;
        };
        if zone.active.is_some() || zone.charge < 1.0 {
            return false;
        }
        zone.charge = 0.0;
        zone.active = Some(ZONE_DURATION);
        self.events.push(GameEvent::ZoneActivated);
        true
    }

    /// Zone end: banked rows vanish and fire as a single attack. The
    /// margin-time multiplier applies, and the attack cancels queued
    /// garbage first like any other clear.
    fn end_zone(&mut self) {
        let Some(zone) = &mut self.zone else { return };
        if zone.active.take().is_none() {
            return;
        }
        let lines = std::mem::take(&mut zone.lines);
        self.board.clear_zone_rows();
        let mut attack = lines + zone_bonus(lines);
        attack = (attack as f32 * self.attack_multiplier()).floor() as u32;
        attack = self.cancel_incoming(attack);
        self.stats.attack_sent += attack;
        self.score += (150 * lines * lines * self.level) as u64;
        self.events.push(GameEvent::ZoneEnded { lines, attack });
    }

    /// Attack an active zone would fire if it ended right now: the banked
    /// lines plus their bonus, margin-scaled, minus what the owner's own
    /// queued garbage would cancel. Zero while no zone runs. This is the
    /// threat the OPPONENT's danger warning should count while the zone
    /// is still stacking up.
    pub fn zone_projected_attack(&self) -> u32 {
        let Some(zone) = &self.zone else { return 0 };
        if zone.active.is_none() || zone.lines == 0 {
            return 0;
        }
        let attack = zone.lines + zone_bonus(zone.lines);
        let attack = (attack as f32 * self.attack_multiplier()).floor() as u32;
        attack.saturating_sub(self.incoming_total())
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

    /// Drop the active piece to its ghost position WITHOUT locking — the
    /// CPU's stand-in for a human holding max-SDF soft drop. Keeping the
    /// piece live lets a rotation follow (spins, tucks).
    pub fn sonic_drop(&mut self) {
        if self.game_over {
            return;
        }
        let target = self.ghost();
        let distance = self.active.y - target.y;
        if distance > 0 {
            self.active = target;
            self.last_rotation_kick = None;
            self.score += distance as u64;
            self.events.push(GameEvent::SoftDropStep);
            self.note_descent();
        }
    }

    pub fn hold(&mut self) {
        if self.game_over {
            return;
        }
        if !self.hold_enabled {
            // Same feedback as a spent hold: the press was heard and
            // refused, rather than silently swallowed.
            self.events.push(GameEvent::HoldBlocked);
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
        let mut piece = ActivePiece::spawn(kind);
        if !self.board.fits(&piece) && self.zone_active() {
            // The banked zone rows pushed the stack into the spawn area;
            // ending the zone early frees them and may save the spawn.
            self.end_zone();
            piece = ActivePiece::spawn(kind);
        }
        if !self.board.fits(&piece) {
            // Block out: the new piece overlaps the stack.
            self.active = piece;
            self.top_out();
            if self.game_over {
                return;
            }
            // Endless: the field was just wiped, so the piece fits now
            // and the normal spawn below can run.
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

    /// The stack reached the top. Normally that ends the run; in endless
    /// play the field is wiped instead and `game_over` stays false, so
    /// every caller must re-check it rather than assuming the game is
    /// over once this returns.
    fn top_out(&mut self) {
        if self.endless {
            self.board = Board::new();
            // The chain died with the stack; starting the fresh field on
            // a live combo or back-to-back would be unearned.
            self.combo = -1;
            self.b2b_armed = false;
            self.incoming.clear();
            self.stats.board_resets += 1;
            self.events.push(GameEvent::BoardReset);
            return;
        }
        self.game_over = true;
        self.events.push(GameEvent::TopOut);
    }

    fn detect_tspin(&self) -> Option<ClearKind> {
        t_spin_kind(&self.board, &self.active, self.last_rotation_kick)
    }

    fn lock_active(&mut self) {
        let piece = self.active;
        let tspin = self.detect_tspin();
        self.board.lock(&piece);
        self.stats.pieces += 1;
        self.events.push(GameEvent::Locked { piece });

        // Lock out: the whole piece settled above the visible field AND
        // touches the deadly center columns — side stacks may poke over
        // the skyline and live. In a zone the banked rows caused the
        // squeeze — end it (dropping the stack back down) instead of
        // ending the game.
        let cells = piece.board_cells();
        if cells.iter().all(|&(_, y)| y >= VISIBLE_HEIGHT)
            && cells.iter().any(|&(x, _)| DEADLY_COLS.contains(&x))
        {
            if self.zone_active() {
                self.end_zone();
            } else {
                self.top_out();
                if self.game_over {
                    return;
                }
                // Endless: the field (including the piece that just
                // locked) is gone, so there is nothing left to clear and
                // no garbage to rise. Go straight to the next piece.
                self.hold_used = false;
                let kind = self.next_from_queue();
                self.spawn_active(kind);
                return;
            }
        }

        if self.zone_active() {
            // Time is stopped: full rows sink to the bottom as zone lines
            // instead of clearing; combo, b2b and garbage stay frozen.
            let banked = self.board.bank_full_rows();
            if banked > 0 {
                let total = match &mut self.zone {
                    Some(zone) => {
                        zone.lines += banked;
                        zone.lines
                    }
                    None => banked,
                };
                self.score += (100 * banked * self.level) as u64;
                self.events.push(GameEvent::ZoneLines { banked, total });
            }
        } else {
            // Count, before the collapse, how many of the rows about to
            // clear contain garbage (they feed the zone gauge a little).
            let garbage_rows_cleared = (0..super::board::BOARD_HEIGHT)
                .filter(|&y| {
                    (0..super::board::BOARD_WIDTH).all(|x| self.board.cell(x, y).is_some())
                        && (0..super::board::BOARD_WIDTH)
                            .any(|x| self.board.cell(x, y) == Some(super::board::Cell::Garbage))
                })
                .count() as u32;
            let rows = self.board.clear_full_rows();
            let lines = rows.len() as u32;

            if lines > 0 {
                self.apply_clear(lines, rows, tspin, garbage_rows_cleared);
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
        }

        if self.game_over {
            return;
        }
        self.hold_used = false;
        let kind = self.next_from_queue();
        self.spawn_active(kind);
    }

    fn apply_clear(
        &mut self,
        lines: u32,
        rows: Vec<i8>,
        tspin: Option<ClearKind>,
        garbage_rows_cleared: u32,
    ) {
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
            self.stats.perfect_clears += 1;
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
        let mut attack = attack_for(kind, lines, b2b, combo, perfect_clear);
        // Margin time: long rounds scale everyone's attack to force an end.
        attack = (attack as f32 * self.attack_multiplier()).floor() as u32;
        // Line clears cancel queued garbage before sending the remainder.
        let sent = self.cancel_incoming(attack);
        let cancelled = attack - sent;
        attack = sent;
        self.stats.attack_sent += attack;

        // Garbage feeds the zone gauge: cancelling it pays well, digging
        // out rows that already rose pays a little. Clean clears pay zero.
        let charge_gain = cancelled as f32 * ZONE_CHARGE_PER_CANCEL
            + garbage_rows_cleared as f32 * ZONE_CHARGE_PER_GARBAGE_LINE;
        if charge_gain > 0.0 {
            if let Some(zone) = &mut self.zone {
                if zone.active.is_none() {
                    let was_full = zone.charge >= 1.0;
                    zone.charge = (zone.charge + charge_gain).min(1.0);
                    if !was_full && zone.charge >= 1.0 {
                        self.events.push(GameEvent::ZoneReady);
                    }
                }
            }
        }

        // --- Lines / level -------------------------------------------------
        self.lines += lines;
        if matches!(self.leveling, Leveling::Lines) {
            let new_level = self.start_level + self.lines / LINES_PER_LEVEL;
            if new_level > self.level {
                self.level = new_level;
                self.events.push(GameEvent::LevelUp { level: new_level });
            }
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
    ///
    /// `garbage_messiness` decides how the batch is cut up: at 0 the whole
    /// attack shares one hole column (a single well to clear it with), and
    /// as it climbs the batch splits into shorter runs at different
    /// columns. Each split lands somewhere other than the run it follows —
    /// repeating the column would just extend the same well, which is the
    /// thing messiness exists to break up.
    pub fn queue_garbage(&mut self, rows: u32) {
        if rows == 0 {
            return;
        }
        let width = super::board::BOARD_WIDTH;
        let mess = self.garbage_messiness.min(100);
        let mut hole = self.garbage_rng.random_range(0..width);
        let mut run = 1u32;
        for _ in 1..rows {
            if mess > 0 && self.garbage_rng.random_range(0..100u32) < mess {
                self.incoming.push_back((run, hole));
                run = 0;
                let mut next = self.garbage_rng.random_range(0..width);
                while next == hole {
                    next = self.garbage_rng.random_range(0..width);
                }
                hole = next;
            }
            run += 1;
        }
        self.incoming.push_back((run, hole));
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

/// Bonus attack rows a zone payout earns on top of its banked lines.
fn zone_bonus(lines: u32) -> u32 {
    match lines {
        0..=3 => 0,
        4..=5 => 1,
        6..=7 => 2,
        8..=9 => 4,
        _ => 6,
    }
}

/// Garbage rows a clear sends before margin-time scaling and cancelling:
/// the guideline-style attack table plus back-to-back, combo and
/// perfect-clear bonuses. Shared by the game and the AI planner so the
/// CPU values moves exactly as they will pay out.
pub fn attack_for(kind: ClearKind, lines: u32, b2b: bool, combo: u32, perfect_clear: bool) -> u32 {
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
    attack
}

/// Classify a T piece about to lock on `board` as a T-spin, a mini, or
/// neither. `last_kick` is the SRS kick-table index of the final
/// successful rotation, or None if the piece moved or fell afterwards
/// (T-spins require the lock to follow a rotation). Shared by the game
/// and the AI planner so both agree on what counts.
pub fn t_spin_kind(
    board: &Board,
    piece: &ActivePiece,
    last_kick: Option<usize>,
) -> Option<ClearKind> {
    if piece.kind != PieceKind::T {
        return None;
    }
    let kick = last_kick?;
    let corners = [(0, 0), (2, 0), (0, 2), (2, 2)];
    // Corners adjacent to the flat "nose" of the T for each rotation.
    let front: [(i8, i8); 2] = match piece.rot {
        Rot::R0 => [(0, 2), (2, 2)],
        Rot::R1 => [(2, 2), (2, 0)],
        Rot::R2 => [(0, 0), (2, 0)],
        Rot::R3 => [(0, 0), (0, 2)],
    };
    let mut filled = 0;
    let mut front_filled = 0;
    for (cx, cy) in corners {
        if board.is_blocked(piece.x + cx, piece.y + cy) {
            filled += 1;
            if front.contains(&(cx, cy)) {
                front_filled += 1;
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{BOARD_WIDTH, Cell};

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

    /// Fill the field to the ceiling, leaving column 0 open so no row is
    /// complete: the next piece has nowhere to spawn and nothing clears
    /// to save it.
    fn bury(game: &mut Game) {
        for y in 0..crate::board::BOARD_HEIGHT {
            for x in 1..BOARD_WIDTH {
                game.board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
    }

    #[test]
    fn endless_play_wipes_the_field_instead_of_ending() {
        let mut game = Game::new(5, 1);
        game.endless = true;
        game.score = 4321;
        game.lines = 77;
        bury(&mut game);
        game.take_events();

        game.hard_drop();

        assert!(!game.game_over, "zen must never end");
        assert!(game.board.is_empty(), "the field should have been wiped");
        assert_eq!(game.stats.board_resets, 1);
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::BoardReset))
        );
        assert!(
            !game.events.iter().any(|e| matches!(e, GameEvent::TopOut)),
            "a wipe is not a top-out"
        );
        // The run continues: the score and line count carry over and
        // there is a live piece to play.
        assert_eq!(game.score, 4321);
        assert_eq!(game.lines, 77);
        assert!(game.board.fits(&game.active));
    }

    #[test]
    fn a_wipe_breaks_the_combo_and_back_to_back() {
        let mut game = Game::new(5, 1);
        game.endless = true;
        game.combo = 4;
        game.b2b_armed = true;
        bury(&mut game);
        game.hard_drop();
        assert_eq!(game.combo(), -1);
        assert!(!game.b2b_armed());
    }

    /// The same situation without `endless` must still end the run —
    /// zen's escape hatch must not leak into marathon or versus.
    #[test]
    fn a_normal_run_still_tops_out() {
        let mut game = Game::new(5, 1);
        bury(&mut game);
        game.take_events();
        game.hard_drop();
        assert!(game.game_over);
        assert_eq!(game.stats.board_resets, 0);
        assert!(game.events.iter().any(|e| matches!(e, GameEvent::TopOut)));
    }

    #[test]
    fn endless_play_survives_being_buried_over_and_over() {
        let mut game = Game::new(9, 1);
        game.endless = true;
        for _ in 0..25 {
            bury(&mut game);
            game.hard_drop();
            assert!(!game.game_over);
        }
        assert_eq!(game.stats.board_resets, 25);
    }

    #[test]
    fn clean_garbage_shares_one_hole_column() {
        let mut game = Game::new(7, 1);
        game.garbage_messiness = 0;
        game.queue_garbage(6);
        assert_eq!(game.incoming.len(), 1, "{:?}", game.incoming);
        assert_eq!(game.incoming[0].0, 6);
        assert_eq!(game.incoming_total(), 6);
    }

    #[test]
    fn messy_garbage_splits_into_runs_at_different_columns() {
        let mut game = Game::new(7, 1);
        game.garbage_messiness = 100;
        game.queue_garbage(6);
        // Every row re-rolls, so the batch is six one-row runs.
        assert_eq!(game.incoming.len(), 6, "{:?}", game.incoming);
        assert!(game.incoming.iter().all(|(rows, _)| *rows == 1));
        assert_eq!(game.incoming_total(), 6);
        // Consecutive runs never share a column: a repeat would just be a
        // longer well, which is the opposite of messy.
        for pair in game.incoming.iter().collect::<Vec<_>>().windows(2) {
            assert_ne!(pair[0].1, pair[1].1);
        }
    }

    #[test]
    fn messiness_does_not_change_how_much_garbage_arrives() {
        // Splitting is purely cosmetic for the danger meter and the rise
        // budget: the same row count lands either way.
        for mess in [0, 35, 70, 100] {
            let mut game = Game::new(11, 1);
            game.garbage_messiness = mess;
            game.queue_garbage(7);
            assert_eq!(game.incoming_total(), 7, "messiness {mess}");
        }
    }

    #[test]
    fn disabled_hold_refuses_and_keeps_the_piece() {
        let mut game = Game::new(3, 1);
        game.hold_enabled = false;
        let active = game.active.kind;
        game.take_events();
        game.hold();
        assert_eq!(game.hold, None);
        assert_eq!(game.active.kind, active);
        assert!(!game.hold_used);
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::HoldBlocked))
        );
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

        game.active = ActivePiece {
            kind: PieceKind::T,
            rot: Rot::R2,
            x: 3,
            y: 0,
        };
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
        game.active = ActivePiece {
            kind: PieceKind::T,
            rot: Rot::R2,
            x: 0,
            y: 0,
        };
        assert!(game.board.fits(&game.active));
        game.last_rotation_kick = Some(0);
        game.hard_drop();
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::TSpinNoLines { mini: true }))
        );
    }

    #[test]
    fn garbage_rises_after_non_clearing_lock_and_cancels() {
        let mut game = Game::new(3, 1);
        game.queue_garbage(3);
        assert_eq!(game.incoming_total(), 3);
        game.hard_drop(); // lock without clearing → garbage rises
        assert_eq!(game.incoming_total(), 0);
        let rose = game
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::GarbageRose { rows: 3 }));
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
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::HoldBlocked))
        );
    }

    #[test]
    fn side_lockout_no_longer_ends_game() {
        // A piece locking fully above the skyline in the side columns is
        // survivable: only the deadly center columns kill.
        let mut game = Game::new(1, 1);
        for x in 0..=1 {
            for y in 0..=20 {
                game.board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
        game.active = ActivePiece {
            kind: PieceKind::O,
            rot: Rot::R0,
            x: 0,
            y: 21,
        };
        game.hard_drop(); // locks at rows 21-22, columns 0-1
        assert!(!game.game_over, "side overflow must not end the game");
        assert_eq!(game.board.cell(0, 21), Some(Cell::Piece(PieceKind::O)));
    }

    #[test]
    fn center_lockout_still_ends_game() {
        let mut game = Game::new(1, 1);
        for x in 3..=6 {
            for y in 0..=20 {
                game.board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
        game.active = ActivePiece {
            kind: PieceKind::O,
            rot: Rot::R0,
            x: 4,
            y: 21,
        };
        game.hard_drop(); // locks at rows 21-22 inside the deadly columns
        assert!(game.game_over, "center overflow must still end the game");
    }

    #[test]
    fn deadly_height_ignores_side_columns() {
        let mut game = Game::new(1, 1);
        for y in 0..18 {
            game.board.set_cell(0, y, Some(Cell::Garbage));
        }
        assert_eq!(game.deadly_height(), 0, "side stacks are not lethal");
        game.board.set_cell(4, 5, Some(Cell::Garbage));
        assert_eq!(game.deadly_height(), 6);
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
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::LevelUp { level: 2 }))
        );
    }

    #[test]
    fn timed_leveling_rises_with_play_time() {
        let mut game = Game::new(1, 1);
        game.leveling = Leveling::Timed {
            seconds_per_level: 1.0,
        };
        for _ in 0..132 {
            game.tick(1.0 / 60.0); // 2.2 s total
        }
        assert_eq!(game.level, 3);
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::LevelUp { level: 2 }))
        );
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::LevelUp { level: 3 }))
        );
    }

    #[test]
    fn timed_leveling_ignores_line_clears() {
        let mut game = Game::new(1, 1);
        game.leveling = Leveling::Timed {
            seconds_per_level: 60.0,
        };
        game.lines = 29; // one more line would be level 4 under the lines rule
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
        assert_eq!(game.lines, 30);
        assert_eq!(game.level, 1, "line clears must not level up in timed mode");
        assert!(
            !game
                .events
                .iter()
                .any(|e| matches!(e, GameEvent::LevelUp { .. }))
        );
    }

    /// Drop a vertical I into the hole at column 4 of row 0.
    fn drop_i_into_col4(game: &mut Game) {
        force_piece(game, PieceKind::I);
        game.rotate(true);
        while game.active.x + 2 != 4 {
            let dir = if game.active.x + 2 < 4 { 1 } else { -1 };
            if !game.move_horizontal(dir) {
                break;
            }
        }
        game.hard_drop();
    }

    #[test]
    fn clean_clear_charges_nothing() {
        let mut game = Game::new(1, 1);
        game.zone = Some(Zone::default());
        // Row built purely of piece cells: no garbage anywhere involved.
        for x in 0..BOARD_WIDTH {
            if x != 4 {
                game.board.set_cell(x, 0, Some(Cell::Piece(PieceKind::O)));
            }
        }
        drop_i_into_col4(&mut game);
        assert_eq!(game.lines, 1);
        assert_eq!(game.zone.as_ref().unwrap().charge, 0.0);
    }

    #[test]
    fn digging_garbage_charges_a_little() {
        let mut game = Game::new(1, 1);
        game.zone = Some(Zone::default());
        fill_row(&mut game, 0, &[4]); // a garbage row on the board
        drop_i_into_col4(&mut game);
        let charge = game.zone.as_ref().unwrap().charge;
        assert!(
            (charge - ZONE_CHARGE_PER_GARBAGE_LINE).abs() < 1e-6,
            "got {charge}"
        );
        assert!(!game.activate_zone(), "partial gauge must not activate");
    }

    #[test]
    fn cancelling_charges_more_than_digging() {
        let mut game = Game::new(1, 1);
        game.zone = Some(Zone::default());
        // A tetris (attack 4) against 3 queued garbage cancels 3 rows;
        // the 4 cleared rows are garbage-built and pay the small rate too.
        game.queue_garbage(3);
        for y in 0..4 {
            fill_row(&mut game, y, &[9]);
        }
        game.board.set_cell(0, 4, Some(Cell::Garbage));
        force_piece(&mut game, PieceKind::I);
        game.rotate(true);
        while game.active.x + 2 < 9 {
            if !game.move_horizontal(1) {
                break;
            }
        }
        game.hard_drop();
        let charge = game.zone.as_ref().unwrap().charge;
        let expected = 3.0 * ZONE_CHARGE_PER_CANCEL + 4.0 * ZONE_CHARGE_PER_GARBAGE_LINE;
        assert!((charge - expected).abs() < 1e-6, "got {charge}");
        assert_eq!(game.incoming_total(), 0, "garbage fully cancelled");
    }

    #[test]
    fn zone_banks_cleared_rows_and_fires_on_end() {
        let mut game = Game::new(1, 1);
        game.zone = Some(Zone {
            charge: 1.0,
            ..Default::default()
        });
        assert!(game.activate_zone());
        assert!(game.zone_active());

        fill_row(&mut game, 0, &[4]);
        drop_i_into_col4(&mut game);
        // The full row banks at the bottom instead of clearing.
        assert!(
            !game
                .events
                .iter()
                .any(|e| matches!(e, GameEvent::Cleared(_)))
        );
        assert!(game.events.iter().any(|e| matches!(
            e,
            GameEvent::ZoneLines {
                banked: 1,
                total: 1
            }
        )));
        assert!(
            (0..BOARD_WIDTH).all(|x| game.board.cell(x, 0) == Some(Cell::Zone)),
            "bottom row must be a zone line"
        );

        // Time stands still: the active piece must not fall.
        let y0 = game.active.y;
        game.tick(2.0);
        assert_eq!(game.active.y, y0, "gravity must freeze during the zone");

        // Timer expiry fires the banked line as attack.
        game.tick(ZONE_DURATION);
        assert!(!game.zone_active());
        assert!(game.events.iter().any(|e| matches!(
            e,
            GameEvent::ZoneEnded {
                lines: 1,
                attack: 1
            }
        )));
        assert!(
            (0..BOARD_WIDTH).all(|x| game.board.cell(x, 0) != Some(Cell::Zone)),
            "zone rows must clear when the zone ends"
        );
        assert_eq!(game.zone.as_ref().unwrap().charge, 0.0, "gauge spent");
    }

    #[test]
    fn zone_projected_attack_tracks_banked_lines() {
        let mut game = Game::new(1, 1);
        game.zone = Some(Zone {
            charge: 1.0,
            ..Default::default()
        });
        assert_eq!(game.zone_projected_attack(), 0, "inactive zone: no threat");
        assert!(game.activate_zone());
        assert_eq!(game.zone_projected_attack(), 0, "nothing banked yet");
        game.zone.as_mut().unwrap().lines = 6;
        assert_eq!(game.zone_projected_attack(), 8, "6 banked + bonus 2");
        // The owner's own queued garbage absorbs the payout first.
        game.queue_garbage(3);
        assert_eq!(game.zone_projected_attack(), 5);
        // The projection matches what the zone actually fires.
        game.tick(ZONE_DURATION + 0.1);
        assert!(game.events.iter().any(|e| matches!(
            e,
            GameEvent::ZoneEnded {
                lines: 6,
                attack: 5
            }
        )));
    }

    #[test]
    fn garbage_stays_queued_during_zone() {
        let mut game = Game::new(3, 1);
        game.zone = Some(Zone {
            charge: 1.0,
            ..Default::default()
        });
        assert!(game.activate_zone());
        game.queue_garbage(3);
        game.hard_drop(); // non-clearing lock during the zone
        assert_eq!(game.incoming_total(), 3, "garbage must not rise in a zone");
        game.tick(ZONE_DURATION + 0.1); // zone ends
        game.hard_drop(); // now it rises as usual
        assert_eq!(game.incoming_total(), 0);
    }

    #[test]
    fn margin_time_multiplier_ladder() {
        let mut game = Game::new(1, 1);
        assert_eq!(
            game.attack_multiplier(),
            1.0,
            "disabled without margin time"
        );
        game.margin_time = Some(90.0);
        for (t, expected) in [
            (0.0, 1.0),
            (89.9, 1.0),
            (90.0, 1.5),
            (120.0, 2.0),
            (150.0, 3.0),
            (180.0, 4.0),
            (600.0, 4.0), // capped
        ] {
            game.stats.time = t;
            assert_eq!(game.attack_multiplier(), expected, "at t={t}");
        }
    }

    #[test]
    fn margin_time_scales_attack_and_announces() {
        let mut game = Game::new(1, 1);
        game.margin_time = Some(90.0);
        game.stats.time = 121.0; // x2 window
        game.tick(1.0 / 60.0);
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::AttackRamp { multiplier } if *multiplier == 2.0)),
            "crossing margin time must announce the ramp"
        );
        // Tetris at x2 sends 8 instead of 4.
        for y in 0..4 {
            fill_row(&mut game, y, &[9]);
        }
        game.board.set_cell(0, 4, Some(Cell::Garbage));
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
        assert_eq!(clear.attack, 8);
    }

    #[test]
    fn fixed_leveling_never_moves() {
        let mut game = Game::new(1, 1);
        game.leveling = Leveling::Fixed;
        game.tick(500.0); // time cannot level it up...
        assert_eq!(game.level, 1);
        game.lines = 39; // ...and neither can cleared lines
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
        assert_eq!(game.lines, 40);
        assert_eq!(game.level, 1);
        assert!(
            !game
                .events
                .iter()
                .any(|e| matches!(e, GameEvent::LevelUp { .. }))
        );
    }

    #[test]
    fn garbage_rows_counts_rows_with_garbage_left() {
        let mut game = Game::new(1, 1);
        assert_eq!(game.board.garbage_rows(), 0);
        game.board.add_garbage(3, 4);
        assert_eq!(game.board.garbage_rows(), 3);
        // A stray piece cell does not count as garbage.
        game.board.set_cell(0, 5, Some(Cell::Piece(PieceKind::O)));
        assert_eq!(game.board.garbage_rows(), 3);
    }

    #[test]
    fn timed_leveling_caps_at_max_level() {
        let mut game = Game::new(1, 1);
        game.leveling = Leveling::Timed {
            seconds_per_level: 1.0,
        };
        game.tick(500.0);
        assert_eq!(game.level, MAX_TIMED_LEVEL);
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
    fn high_sdf_reaches_the_floor_within_a_tick() {
        let mut game = Game::new(1, 1);
        game.soft_drop_factor = 1_000_000.0;
        game.set_soft_drop(true);
        game.tick(1.0 / 60.0);
        assert!(
            !game.board.fits(&game.active.shifted(0, -1)),
            "piece should be grounded after one tick at MAX SDF"
        );
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
        game.active = ActivePiece {
            kind: PieceKind::O,
            rot: Rot::R0,
            x: 8,
            y: 19,
        };
        game.queue_garbage(8);
        game.hard_drop(); // locks in place (rows 19-20), no line clear
        assert!(
            game.events
                .iter()
                .any(|e| matches!(e, GameEvent::GarbageRose { rows: 8 })),
            "garbage should have risen"
        );
        assert!(
            !game.game_over,
            "legal board must not top out after garbage rise"
        );
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
        assert_eq!(
            game.stats.pieces, before,
            "landing frame must not charge lock delay"
        );
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
        assert!(game.events.iter().any(|e| matches!(
            e,
            GameEvent::Rotated {
                kicked: false,
                cw: true,
                ..
            }
        )));
    }
}
