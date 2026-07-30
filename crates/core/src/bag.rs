//! Guideline 7-bag randomizer: each permutation of the seven tetrominoes is
//! dealt completely before the next bag begins.

use super::piece::PieceKind;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

#[derive(Debug, Clone)]
pub struct SevenBag {
    rng: StdRng,
    /// Remaining pieces of the current bag, dealt from the back.
    bag: Vec<PieceKind>,
}

impl SevenBag {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            bag: Vec::with_capacity(7),
        }
    }

    pub fn from_entropy() -> Self {
        Self::new(rand::rng().random())
    }

    /// Deal the next piece. Not `next`, and not an `Iterator`: this stream
    /// never ends, and an `Option` that is always `Some` would be an unwrap
    /// at every call site for nothing.
    pub fn next_piece(&mut self) -> PieceKind {
        if self.bag.is_empty() {
            self.bag.extend_from_slice(&PieceKind::ALL);
            self.bag.shuffle(&mut self.rng);
        }
        self.bag.pop().expect("bag was just refilled")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_bag_contains_all_seven_pieces() {
        let mut bag = SevenBag::new(42);
        for _ in 0..20 {
            let drawn: HashSet<PieceKind> = (0..7).map(|_| bag.next_piece()).collect();
            assert_eq!(drawn.len(), 7);
        }
    }

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SevenBag::new(7);
        let mut b = SevenBag::new(7);
        for _ in 0..70 {
            assert_eq!(a.next_piece(), b.next_piece());
        }
    }
}
