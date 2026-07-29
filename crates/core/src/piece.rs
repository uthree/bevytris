//! Tetromino definitions: kinds, rotation states, cell layouts and colors.
//!
//! Coordinate system: `x` grows rightward, `y` grows upward. Each piece lives
//! in a square bounding box (2x2 for O, 4x4 for I, 3x3 for the rest) as
//! required by the Super Rotation System.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PieceKind {
    I,
    O,
    T,
    S,
    Z,
    J,
    L,
}

impl PieceKind {
    pub const ALL: [PieceKind; 7] = [
        PieceKind::I,
        PieceKind::O,
        PieceKind::T,
        PieceKind::S,
        PieceKind::Z,
        PieceKind::J,
        PieceKind::L,
    ];

    pub fn box_size(self) -> i8 {
        match self {
            PieceKind::I => 4,
            PieceKind::O => 2,
            _ => 3,
        }
    }

    /// Guideline colors as linear-ish RGB triples (0.0..=1.0).
    pub fn color(self) -> (f32, f32, f32) {
        match self {
            PieceKind::I => (0.00, 0.80, 0.95), // cyan
            PieceKind::O => (0.95, 0.80, 0.00), // yellow
            PieceKind::T => (0.63, 0.20, 0.80), // purple
            PieceKind::S => (0.20, 0.85, 0.25), // green
            PieceKind::Z => (0.90, 0.15, 0.20), // red
            PieceKind::J => (0.15, 0.30, 0.90), // blue
            PieceKind::L => (0.95, 0.50, 0.05), // orange
        }
    }

    /// Cells occupied in the spawn orientation, as offsets inside the
    /// bounding box (origin at the box's bottom-left corner).
    fn spawn_cells(self) -> [(i8, i8); 4] {
        match self {
            PieceKind::I => [(0, 2), (1, 2), (2, 2), (3, 2)],
            PieceKind::O => [(0, 0), (1, 0), (0, 1), (1, 1)],
            PieceKind::T => [(0, 1), (1, 1), (2, 1), (1, 2)],
            PieceKind::S => [(0, 1), (1, 1), (1, 2), (2, 2)],
            PieceKind::Z => [(0, 2), (1, 2), (1, 1), (2, 1)],
            PieceKind::J => [(0, 2), (0, 1), (1, 1), (2, 1)],
            PieceKind::L => [(2, 2), (0, 1), (1, 1), (2, 1)],
        }
    }
}

/// Rotation state per SRS: 0 = spawn, R = one clockwise turn,
/// 2 = two turns, L = one counter-clockwise turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Rot {
    #[default]
    R0,
    R1,
    R2,
    R3,
}

impl Rot {
    pub const ALL: [Rot; 4] = [Rot::R0, Rot::R1, Rot::R2, Rot::R3];

    pub fn cw(self) -> Rot {
        match self {
            Rot::R0 => Rot::R1,
            Rot::R1 => Rot::R2,
            Rot::R2 => Rot::R3,
            Rot::R3 => Rot::R0,
        }
    }

    pub fn ccw(self) -> Rot {
        match self {
            Rot::R0 => Rot::R3,
            Rot::R1 => Rot::R0,
            Rot::R2 => Rot::R1,
            Rot::R3 => Rot::R2,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Rot::R0 => 0,
            Rot::R1 => 1,
            Rot::R2 => 2,
            Rot::R3 => 3,
        }
    }
}

/// Cells occupied by `kind` at rotation `rot`, as offsets inside the
/// bounding box. Rotation is performed inside the box, exactly as SRS
/// specifies (a clockwise turn maps (x, y) -> (y, n-1-x)).
pub fn cells(kind: PieceKind, rot: Rot) -> [(i8, i8); 4] {
    let n = kind.box_size();
    let mut cs = kind.spawn_cells();
    for _ in 0..rot.index() {
        for c in cs.iter_mut() {
            *c = (c.1, n - 1 - c.0);
        }
    }
    cs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn set(cs: [(i8, i8); 4]) -> HashSet<(i8, i8)> {
        cs.into_iter().collect()
    }

    #[test]
    fn t_piece_rotations() {
        assert_eq!(
            set(cells(PieceKind::T, Rot::R0)),
            set([(0, 1), (1, 1), (2, 1), (1, 2)])
        );
        // Pointing right after one clockwise turn.
        assert_eq!(
            set(cells(PieceKind::T, Rot::R1)),
            set([(1, 0), (1, 1), (1, 2), (2, 1)])
        );
        // Pointing down.
        assert_eq!(
            set(cells(PieceKind::T, Rot::R2)),
            set([(0, 1), (1, 1), (2, 1), (1, 0)])
        );
        // Pointing left.
        assert_eq!(
            set(cells(PieceKind::T, Rot::R3)),
            set([(1, 0), (1, 1), (1, 2), (0, 1)])
        );
    }

    #[test]
    fn i_piece_rotations() {
        assert_eq!(
            set(cells(PieceKind::I, Rot::R0)),
            set([(0, 2), (1, 2), (2, 2), (3, 2)])
        );
        // Vertical, in the third column of the 4-box.
        assert_eq!(
            set(cells(PieceKind::I, Rot::R1)),
            set([(2, 0), (2, 1), (2, 2), (2, 3)])
        );
        assert_eq!(
            set(cells(PieceKind::I, Rot::R2)),
            set([(0, 1), (1, 1), (2, 1), (3, 1)])
        );
        assert_eq!(
            set(cells(PieceKind::I, Rot::R3)),
            set([(1, 0), (1, 1), (1, 2), (1, 3)])
        );
    }

    #[test]
    fn o_piece_rotation_invariant() {
        for rot in Rot::ALL {
            assert_eq!(
                set(cells(PieceKind::O, rot)),
                set(cells(PieceKind::O, Rot::R0))
            );
        }
    }

    #[test]
    fn four_turns_return_to_spawn() {
        for kind in PieceKind::ALL {
            let mut rot = Rot::R0;
            for _ in 0..4 {
                rot = rot.cw();
            }
            assert_eq!(rot, Rot::R0);
            assert_eq!(set(cells(kind, Rot::R0)), set(cells(kind, rot)));
        }
    }
}
