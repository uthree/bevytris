//! Super Rotation System wall-kick tables.
//!
//! Offsets are (x, y) with +y pointing up, applied to the piece's bounding
//! box position. Tables follow the standard SRS specification: five tests
//! per rotation for J/L/S/T/Z and I; the O piece does not kick.

use super::piece::{PieceKind, Rot};

/// Wall-kick test offsets for rotating `kind` from `from` to `to`.
/// `to` must be one clockwise or counter-clockwise step from `from`.
pub fn kicks(kind: PieceKind, from: Rot, to: Rot) -> &'static [(i8, i8)] {
    match kind {
        PieceKind::O => &[(0, 0)],
        PieceKind::I => kicks_i(from, to),
        _ => kicks_jlstz(from, to),
    }
}

fn kicks_jlstz(from: Rot, to: Rot) -> &'static [(i8, i8)] {
    use Rot::*;
    match (from, to) {
        (R0, R1) => &[(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
        (R1, R0) => &[(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
        (R1, R2) => &[(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
        (R2, R1) => &[(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
        (R2, R3) => &[(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
        (R3, R2) => &[(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
        (R3, R0) => &[(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
        (R0, R3) => &[(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
        _ => panic!("invalid SRS rotation {from:?} -> {to:?}"),
    }
}

fn kicks_i(from: Rot, to: Rot) -> &'static [(i8, i8)] {
    use Rot::*;
    match (from, to) {
        (R0, R1) => &[(0, 0), (-2, 0), (1, 0), (-2, -1), (1, 2)],
        (R1, R0) => &[(0, 0), (2, 0), (-1, 0), (2, 1), (-1, -2)],
        (R1, R2) => &[(0, 0), (-1, 0), (2, 0), (-1, 2), (2, -1)],
        (R2, R1) => &[(0, 0), (1, 0), (-2, 0), (1, -2), (-2, 1)],
        (R2, R3) => &[(0, 0), (2, 0), (-1, 0), (2, 1), (-1, -2)],
        (R3, R2) => &[(0, 0), (-2, 0), (1, 0), (-2, -1), (1, 2)],
        (R3, R0) => &[(0, 0), (1, 0), (-2, 0), (1, -2), (-2, 1)],
        (R0, R3) => &[(0, 0), (-1, 0), (2, 0), (-1, 2), (2, -1)],
        _ => panic!("invalid SRS rotation {from:?} -> {to:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tables_have_five_tests_and_start_at_origin() {
        for kind in [PieceKind::T, PieceKind::I] {
            for from in Rot::ALL {
                for to in [from.cw(), from.ccw()] {
                    let table = kicks(kind, from, to);
                    assert_eq!(table.len(), 5, "{kind:?} {from:?}->{to:?}");
                    assert_eq!(table[0], (0, 0));
                }
            }
        }
    }

    #[test]
    fn jlstz_reverse_kicks_are_negated() {
        // SRS property: the table for B->A is the negation of A->B.
        for from in Rot::ALL {
            for to in [from.cw(), from.ccw()] {
                let fwd = kicks(PieceKind::T, from, to);
                let rev = kicks(PieceKind::T, to, from);
                for (f, r) in fwd.iter().zip(rev.iter()) {
                    assert_eq!((f.0 + r.0, f.1 + r.1), (0, 0), "{from:?}->{to:?}");
                }
            }
        }
    }

    #[test]
    fn i_reverse_kicks_are_negated() {
        for from in Rot::ALL {
            for to in [from.cw(), from.ccw()] {
                let fwd = kicks(PieceKind::I, from, to);
                let rev = kicks(PieceKind::I, to, from);
                for (f, r) in fwd.iter().zip(rev.iter()) {
                    assert_eq!((f.0 + r.0, f.1 + r.1), (0, 0), "{from:?}->{to:?}");
                }
            }
        }
    }
}
