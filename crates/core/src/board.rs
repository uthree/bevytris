//! The playfield: a 10x40 grid of which the bottom 20 rows are visible.

use super::piece::{PieceKind, Rot, cells};

pub const BOARD_WIDTH: i8 = 10;
pub const BOARD_HEIGHT: i8 = 40;
pub const VISIBLE_HEIGHT: i8 = 20;

/// Contents of a single locked cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// A banked line from an active zone, parked at the bottom of the
    /// field until the zone ends.
    Zone,
    Piece(PieceKind),
    Garbage,
}

/// An active (falling) piece: kind, rotation and bounding-box position.
/// `x`/`y` locate the box's bottom-left corner on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivePiece {
    pub kind: PieceKind,
    pub rot: Rot,
    pub x: i8,
    pub y: i8,
}

impl ActivePiece {
    /// Spawn per guideline: boxes are horizontally centered (rounding left)
    /// and the piece occupies rows 20-21, just above the visible field.
    pub fn spawn(kind: PieceKind) -> Self {
        let (x, y) = match kind {
            PieceKind::I => (3, 18),
            PieceKind::O => (4, 20),
            _ => (3, 19),
        };
        ActivePiece {
            kind,
            rot: Rot::R0,
            x,
            y,
        }
    }

    /// Board coordinates of the four occupied cells.
    pub fn board_cells(&self) -> [(i8, i8); 4] {
        let mut cs = cells(self.kind, self.rot);
        for c in cs.iter_mut() {
            *c = (c.0 + self.x, c.1 + self.y);
        }
        cs
    }

    pub fn shifted(&self, dx: i8, dy: i8) -> Self {
        ActivePiece {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    pub fn rotated(&self, rot: Rot) -> Self {
        ActivePiece { rot, ..*self }
    }
}

#[derive(Debug, Clone)]
pub struct Board {
    /// `grid[y][x]`, row 0 at the bottom.
    grid: Vec<[Option<Cell>; BOARD_WIDTH as usize]>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    pub fn new() -> Self {
        Self {
            grid: vec![[None; BOARD_WIDTH as usize]; BOARD_HEIGHT as usize],
        }
    }

    pub fn cell(&self, x: i8, y: i8) -> Option<Cell> {
        if !(0..BOARD_WIDTH).contains(&x) || !(0..BOARD_HEIGHT).contains(&y) {
            return None;
        }
        self.grid[y as usize][x as usize]
    }

    /// True if (x, y) is outside the field or occupied. Out-of-bounds cells
    /// count as filled, which is exactly what collision and the T-spin
    /// corner rule need.
    pub fn is_blocked(&self, x: i8, y: i8) -> bool {
        if !(0..BOARD_WIDTH).contains(&x) || !(0..BOARD_HEIGHT).contains(&y) {
            return true;
        }
        self.grid[y as usize][x as usize].is_some()
    }

    pub fn fits(&self, piece: &ActivePiece) -> bool {
        piece
            .board_cells()
            .iter()
            .all(|&(x, y)| !self.is_blocked(x, y))
    }

    /// Write the piece's cells into the grid.
    pub fn lock(&mut self, piece: &ActivePiece) {
        for (x, y) in piece.board_cells() {
            if (0..BOARD_WIDTH).contains(&x) && (0..BOARD_HEIGHT).contains(&y) {
                self.grid[y as usize][x as usize] = Some(Cell::Piece(piece.kind));
            }
        }
    }

    /// Remove all full rows, returning their y coordinates (bottom-up order,
    /// coordinates as they were before removal).
    pub fn clear_full_rows(&mut self) -> Vec<i8> {
        let full: Vec<i8> = (0..BOARD_HEIGHT)
            .filter(|&y| self.grid[y as usize].iter().all(|c| c.is_some()))
            .collect();
        // Remove from the top down so lower indices stay valid.
        for &y in full.iter().rev() {
            self.grid.remove(y as usize);
            self.grid.push([None; BOARD_WIDTH as usize]);
        }
        full
    }

    pub fn is_empty(&self) -> bool {
        self.grid.iter().all(|row| row.iter().all(|c| c.is_none()))
    }

    /// Insert `count` garbage rows at the bottom, each with a single hole at
    /// `hole_x`. Existing content shifts up; anything pushed above the field
    /// is discarded.
    pub fn add_garbage(&mut self, count: usize, hole_x: i8) {
        for _ in 0..count {
            let mut row = [Some(Cell::Garbage); BOARD_WIDTH as usize];
            if (0..BOARD_WIDTH).contains(&hole_x) {
                row[hole_x as usize] = None;
            }
            self.grid.pop();
            self.grid.insert(0, row);
        }
    }

    /// Zone banking: remove every full row that holds at least one
    /// non-zone cell, then push the same number of solid zone rows in at
    /// the bottom (existing content shifts up). Rows that are already
    /// pure zone lines don't re-bank. Returns how many rows were banked.
    pub fn bank_full_rows(&mut self) -> u32 {
        let full: Vec<i8> = (0..BOARD_HEIGHT)
            .filter(|&y| {
                let row = &self.grid[y as usize];
                row.iter().all(|c| c.is_some()) && row.iter().any(|c| *c != Some(Cell::Zone))
            })
            .collect();
        for &y in full.iter().rev() {
            self.grid.remove(y as usize);
            self.grid.push([None; BOARD_WIDTH as usize]);
        }
        for _ in 0..full.len() {
            self.grid.pop();
            self.grid
                .insert(0, [Some(Cell::Zone); BOARD_WIDTH as usize]);
        }
        full.len() as u32
    }

    /// Remove the stack of zone rows at the bottom (zone end), letting the
    /// rest of the field drop back down. Returns how many were cleared.
    pub fn clear_zone_rows(&mut self) -> u32 {
        let mut n = 0;
        while self
            .grid
            .first()
            .is_some_and(|row| row.iter().all(|c| *c == Some(Cell::Zone)))
        {
            self.grid.remove(0);
            self.grid.push([None; BOARD_WIDTH as usize]);
            n += 1;
        }
        n
    }

    /// Rows still holding at least one garbage cell — dig mode's clear
    /// target counts rows, matching what the player sees on the field.
    pub fn garbage_rows(&self) -> u32 {
        self.grid
            .iter()
            .filter(|row| row.iter().any(|c| *c == Some(Cell::Garbage)))
            .count() as u32
    }

    /// Height of the tallest filled cell in each column (0 = empty column).
    pub fn column_heights(&self) -> [i8; BOARD_WIDTH as usize] {
        let mut heights = [0i8; BOARD_WIDTH as usize];
        for x in 0..BOARD_WIDTH {
            for y in (0..BOARD_HEIGHT).rev() {
                if self.grid[y as usize][x as usize].is_some() {
                    heights[x as usize] = y + 1;
                    break;
                }
            }
        }
        heights
    }

    /// Direct cell write, used by tests and the AI simulator.
    pub fn set_cell(&mut self, x: i8, y: i8, cell: Option<Cell>) {
        if (0..BOARD_WIDTH).contains(&x) && (0..BOARD_HEIGHT).contains(&y) {
            self.grid[y as usize][x as usize] = cell;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_row(board: &mut Board, y: i8, hole: Option<i8>) {
        for x in 0..BOARD_WIDTH {
            if Some(x) != hole {
                board.set_cell(x, y, Some(Cell::Garbage));
            }
        }
    }

    #[test]
    fn spawn_positions_follow_guideline() {
        // J/L/S/T/Z occupy columns 3-5 in rows 20-21.
        let t = ActivePiece::spawn(PieceKind::T);
        for (x, y) in t.board_cells() {
            assert!((3..=5).contains(&x) && (20..=21).contains(&y));
        }
        // I occupies columns 3-6, row 20.
        let i = ActivePiece::spawn(PieceKind::I);
        for (x, y) in i.board_cells() {
            assert!((3..=6).contains(&x) && y == 20);
        }
        // O occupies columns 4-5, rows 20-21.
        let o = ActivePiece::spawn(PieceKind::O);
        for (x, y) in o.board_cells() {
            assert!((4..=5).contains(&x) && (20..=21).contains(&y));
        }
    }

    #[test]
    fn clear_detects_and_removes_full_rows() {
        let mut board = Board::new();
        fill_row(&mut board, 0, None);
        fill_row(&mut board, 1, Some(4));
        fill_row(&mut board, 2, None);
        board.set_cell(0, 3, Some(Cell::Piece(PieceKind::L)));

        let cleared = board.clear_full_rows();
        assert_eq!(cleared, vec![0, 2]);
        // Row with the hole slid down to the bottom.
        assert_eq!(board.cell(4, 0), None);
        assert_eq!(board.cell(0, 0), Some(Cell::Garbage));
        // Lone cell slid from row 3 to row 1.
        assert_eq!(board.cell(0, 1), Some(Cell::Piece(PieceKind::L)));
        assert_eq!(board.cell(0, 3), None);
    }

    #[test]
    fn garbage_pushes_content_up() {
        let mut board = Board::new();
        board.set_cell(5, 0, Some(Cell::Piece(PieceKind::T)));
        board.add_garbage(2, 3);
        assert_eq!(board.cell(5, 2), Some(Cell::Piece(PieceKind::T)));
        assert_eq!(board.cell(3, 0), None);
        assert_eq!(board.cell(3, 1), None);
        assert_eq!(board.cell(0, 0), Some(Cell::Garbage));
        assert_eq!(board.cell(9, 1), Some(Cell::Garbage));
    }

    #[test]
    fn out_of_bounds_is_blocked() {
        let board = Board::new();
        assert!(board.is_blocked(-1, 0));
        assert!(board.is_blocked(BOARD_WIDTH, 0));
        assert!(board.is_blocked(0, -1));
        assert!(!board.is_blocked(0, 0));
    }
}
